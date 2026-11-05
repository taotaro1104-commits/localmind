//! 永続化・索引の整合（meta）・同時 open・ハイブリッド品質のテスト。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use super::tests::{hashing, lex, loaded, ngram, sem};
use super::*;
use crate::eval::evaluate_searcher;
use crate::grounding::{HybridConfig, HybridSearcher, SearchOutcome, DEFAULT_GROUNDING_THRESHOLD};
use crate::tokenize::NgramTokenizer;
use crate::vector::HashingEmbedder;

pub(super) fn temp_db(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("lm-search-{}-{name}.db", std::process::id()));
    cleanup(&p);
    p
}

pub(super) fn cleanup(p: &Path) {
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", p.display()));
    }
}

pub(super) fn open(path: &Path) -> Result<SqliteStore, SearchError> {
    SqliteStore::open(path, ngram(), hashing(64))
}

// ---- 永続化 --------------------------------------------------------------------------

#[test]
fn data_persists_across_reopen() {
    let path = temp_db("persist");
    {
        let s = open(&path).unwrap();
        s.upsert("a", "永続化のテスト").unwrap();
    }
    let s = open(&path).unwrap();
    assert_eq!(s.len().unwrap(), 1);
    assert_eq!(lex(&s, "永続化", 3)[0].id, "a");
    assert_eq!(sem(&s, "永続化のテスト", 1)[0].id, "a");
    drop(s);
    cleanup(&path);
}

#[test]
fn file_database_is_in_wal_mode_and_two_connections_can_write() {
    let path = temp_db("wal");
    let a = open(&path).unwrap();
    let b = open(&path).unwrap();
    assert_eq!(a.journal_mode().to_lowercase(), "wal");
    assert_eq!(b.journal_mode().to_lowercase(), "wal");
    a.upsert("from-a", "最初の文書です").unwrap();
    b.upsert("from-b", "二番目の文書です").unwrap();
    assert_eq!(a.len().unwrap(), 2);
    assert_eq!(b.len().unwrap(), 2);
    // 一方の更新が他方の語彙・意味検索にも見える。
    assert_eq!(lex(&b, "最初", 3)[0].id, "from-a");
    assert_eq!(sem(&a, "二番目の文書です", 1)[0].id, "from-b");
    drop((a, b));
    cleanup(&path);
}

/// 新規ファイルを複数スレッドが同時に開いても失敗しない（WAL 切替の競合をリトライで吸収する）。
#[test]
fn concurrent_open_of_a_new_database_does_not_fail() {
    const THREADS: usize = 8;
    for round in 0..15 {
        let path = temp_db(&format!("race{round}"));
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let (path, barrier) = (path.clone(), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    barrier.wait();
                    open(&path).map(|_| ())
                })
            })
            .collect();
        for h in handles {
            let r = h.join().expect("スレッドが panic した");
            assert!(r.is_ok(), "round {round}: {r:?}");
        }
        cleanup(&path);
    }
}

// ---- 索引の整合 ----------------------------------------------------------------------

pub(super) fn reopen_err(path: &Path, t: Box<dyn Tokenizer>, e: Box<dyn Embedder>) -> SearchError {
    SqliteStore::open(path, t, e)
        .err()
        .expect("不一致の索引は開けないはず")
}

#[test]
fn reopening_with_a_different_dimension_is_rejected() {
    let path = temp_db("dim");
    open(&path).unwrap();
    let err = reopen_err(&path, ngram(), hashing(128));
    assert!(matches!(err, SearchError::Config(_)), "{err:?}");
    cleanup(&path);
}

struct OtherTokenizer;
impl Tokenizer for OtherTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        NgramTokenizer.tokenize(text)
    }
    fn id(&self) -> String {
        "other-v1".into()
    }
}

struct OtherEmbedder;
impl Embedder for OtherEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        HashingEmbedder::new(64).embed(text)
    }
    fn dim(&self) -> usize {
        64
    }
    fn id(&self) -> String {
        "other-model-v1".into()
    }
}

#[test]
fn reopening_with_a_different_tokenizer_or_embedder_is_rejected() {
    let path = temp_db("identity");
    open(&path).unwrap();
    let e1 = reopen_err(&path, Box::new(OtherTokenizer), hashing(64));
    assert!(
        matches!(&e1, SearchError::Config(m) if m.contains("トークナイザ")),
        "{e1:?}"
    );
    let e2 = reopen_err(&path, ngram(), Box::new(OtherEmbedder));
    assert!(
        matches!(&e2, SearchError::Config(m) if m.contains("埋め込みモデル")),
        "{e2:?}"
    );
    assert!(open(&path).is_ok());
    cleanup(&path);
}

pub(super) fn set_meta(path: &Path, sql: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute(sql, []).unwrap();
}

#[test]
fn reopening_with_a_different_normalize_version_is_rejected() {
    let path = temp_db("normver");
    drop(open(&path).unwrap());
    set_meta(
        &path,
        "UPDATE meta SET value='0' WHERE key='normalize_version'",
    );
    let err = reopen_err(&path, ngram(), hashing(64));
    assert!(
        matches!(&err, SearchError::Config(m) if m.contains("正規化")),
        "{err:?}"
    );
    cleanup(&path);
}

#[test]
fn a_database_with_documents_but_missing_meta_is_rejected_not_silently_adopted() {
    for key in [
        "normalize_version",
        "tokenizer_id",
        "embedder_id",
        "dim",
        "fts_schema",
    ] {
        let path = temp_db(&format!("nometa-{key}"));
        {
            let s = open(&path).unwrap();
            s.upsert("a", "古い規則で索引された文書").unwrap();
        }
        set_meta(&path, &format!("DELETE FROM meta WHERE key='{key}'"));
        let err = reopen_err(&path, ngram(), hashing(64));
        assert!(
            matches!(&err, SearchError::Config(m) if m.contains("欠けて")),
            "{key}: {err:?}"
        );
        cleanup(&path);
    }
}

#[test]
fn an_empty_database_with_missing_meta_is_re_registered() {
    let path = temp_db("nometa-empty");
    drop(open(&path).unwrap());
    set_meta(&path, "DELETE FROM meta WHERE key='normalize_version'");
    let s = open(&path).expect("文書の無い DB は meta を再登録して開ける");
    s.upsert("a", "再登録後の文書").unwrap();
    drop(s);
    assert!(open(&path).is_ok());
    cleanup(&path);
}

// ---- ハイブリッド ---------------------------------------------------------------------

pub(super) fn hybrid(s: SqliteStore, threshold: f32) -> HybridSearcher {
    let s = Arc::new(s);
    HybridSearcher::new(
        s.clone(),
        s,
        HybridConfig {
            grounding_threshold: threshold,
            ..HybridConfig::default()
        },
    )
}

#[test]
fn hybrid_over_sqlite_answers_and_refuses() {
    let h = hybrid(loaded(), DEFAULT_GROUNDING_THRESHOLD);
    match h.search("AB-1234", 3).unwrap() {
        SearchOutcome::Found { hits, .. } => assert_eq!(hits[0].id, "manual-ab1234"),
        other => panic!("{other:?}"),
    }
    for q in ["第99条", "第十三条"] {
        assert!(
            matches!(h.search(q, 3).unwrap(), SearchOutcome::NotFound { .. }),
            "{q}"
        );
    }
}

/// SQLite 版はインメモリ版と同じ品質基準・同じ既定閾値で通ること。
#[test]
fn sqlite_backend_meets_same_quality_bar_as_in_memory() {
    let sqlite = evaluate_searcher(
        &hybrid(loaded(), DEFAULT_GROUNDING_THRESHOLD),
        DEFAULT_GROUNDING_THRESHOLD,
        3,
    );
    let memory = crate::eval::evaluate(DEFAULT_GROUNDING_THRESHOLD, 3);
    assert!(sqlite.recall_at_k >= 0.95, "{sqlite:?}");
    assert_eq!(sqlite.false_accept_rate, 0.0, "{sqlite:?}");
    assert!(sqlite.false_reject_rate <= 0.05, "{sqlite:?}");
    assert!(
        (sqlite.balanced_error() - memory.balanced_error()).abs() < 0.03,
        "sqlite={sqlite:?} memory={memory:?}"
    );
}

/// バンドル版の SQLite は URI 解釈が既定で有効なので、`file:` で始まるパスは絶対パスにして
/// URI として解釈させない。
#[test]
fn a_file_uri_looking_path_is_turned_into_a_plain_absolute_path() {
    for raw in [
        "file:evil.db?mode=memory&cache=shared",
        "file::memory:",
        "x.db",
    ] {
        let p = db_path(Path::new(raw)).unwrap();
        assert!(p.is_absolute(), "{raw} -> {p:?}");
        assert!(!p.to_string_lossy().starts_with("file:"), "{raw} -> {p:?}");
    }
    // メモリ DB と空パスはそのまま。
    assert_eq!(
        db_path(Path::new(":memory:")).unwrap(),
        Path::new(":memory:")
    );
}

#[cfg(unix)]
#[test]
fn a_new_database_file_is_private_and_a_symlinked_path_is_refused() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("lm-db-perm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("index.db");
    drop(SqliteStore::open(&db, ngram(), Box::new(HashingEmbedder::new(16))).unwrap());
    let mode = std::fs::metadata(&db).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let link = dir.join("link.db");
    std::os::unix::fs::symlink(&db, &link).unwrap();
    assert!(SqliteStore::open(&link, ngram(), Box::new(HashingEmbedder::new(16))).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 中身が SQLite ではないファイル（破損・差し替え）は、開く時点でエラーになる。
#[test]
fn a_corrupted_database_file_is_rejected_on_open() {
    let dir = std::env::temp_dir().join(format!("lm-db-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("index.db");
    std::fs::write(&db, vec![0xABu8; 4096]).unwrap();
    assert!(SqliteStore::open(&db, ngram(), Box::new(HashingEmbedder::new(16))).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 全 OS で、ディレクトリなど通常のファイルでないパスは DB として開かない。
#[test]
fn a_directory_is_not_accepted_as_a_database_path() {
    let dir = std::env::temp_dir().join(format!("lm-db-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(SqliteStore::open(&dir, ngram(), Box::new(HashingEmbedder::new(16))).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 内部に不整合（制約違反の行）がある DB は、`quick_check` が開く前に見つけて「破損」として拒否する。
#[test]
fn a_database_failing_quick_check_is_reported_as_corrupted() {
    let dir = std::env::temp_dir().join(format!("lm-db-damaged-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("index.db");
    {
        // CHECK 制約を無視して違反行を入れる（正常な手順では作れない不整合な DB）。
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE t(a INTEGER CHECK(a > 0));
             PRAGMA ignore_check_constraints = ON;
             INSERT INTO t VALUES (-1);",
        )
        .unwrap();
    }
    let err = match SqliteStore::open(&db, ngram(), Box::new(HashingEmbedder::new(16))) {
        Ok(_) => panic!("不整合な DB が開けた"),
        Err(e) => e,
    };
    assert!(
        matches!(&err, SearchError::Storage(m) if m.contains("破損")),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- 索引ファイルの整合・防御 ---------------------------------------------------------

#[test]
fn a_vector_table_with_a_different_dimension_is_refused_at_open() {
    let path = temp_db("vecdim");
    drop(open(&path).unwrap()); // 次元 64 で作成
    {
        // meta は 64 のまま、ベクトル表だけ別の次元で作り直す（細工された索引ファイル）。
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE docs_vec;
             CREATE VIRTUAL TABLE docs_vec USING vec0(embedding float[4] distance_metric=cosine);",
        )
        .unwrap();
    }
    let err = open(&path).err().expect("次元の不一致は拒否される");
    assert!(err.to_string().contains("ベクトル表の次元"), "{err}");
    cleanup(&path);
}

#[test]
fn shadow_tables_cannot_be_written_from_sql() {
    // 防御モードにより、仮想表の裏側の表を SQL から直接書き換えられない。
    let s = SqliteStore::in_memory(ngram(), hashing(64)).unwrap();
    let conn = s.lock();
    let err = conn
        .execute(
            "INSERT INTO docs_fts_data(id, block) VALUES (999999, x'00')",
            [],
        )
        .unwrap_err();
    assert!(err.to_string().contains("may not be modified"), "{err}");
    // 通常の登録・検索は影響を受けない。
    drop(conn);
    s.upsert("a", "防御モードでも動く").unwrap();
    assert_eq!(lex(&s, "防御", 3)[0].id, "a");
}

#[test]
fn the_declared_vector_dimension_is_read_exactly() {
    use super::setup::declared_vec_dim;
    let ok = "CREATE VIRTUAL TABLE docs_vec USING vec0(\n    embedding float[384] distance_metric=cosine)";
    assert_eq!(declared_vec_dim(ok), Some(384));
    assert_eq!(declared_vec_dim(&ok.replace("384", "38")), Some(38));
    // コメント・別の列・複数の宣言・欠落は読み取れない（= 不一致として拒否される）。
    for bad in [
        "CREATE VIRTUAL TABLE docs_vec USING vec0(x float[3], embedding float[384])",
        "CREATE VIRTUAL TABLE docs_vec USING vec0(/*float[384]*/ embedding float[4])",
        "CREATE VIRTUAL TABLE docs_vec USING vec0(embedding float[])",
        "CREATE VIRTUAL TABLE docs_vec USING vec0(embedding float[a4])",
        "CREATE VIRTUAL TABLE docs_vec USING vec0(embedding int8[384])",
        "",
    ] {
        assert_eq!(declared_vec_dim(bad), None, "{bad}");
    }
}
