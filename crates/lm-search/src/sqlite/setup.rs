//! SQLite 索引の初期化: 接続設定・WAL・スキーマ作成・meta 照合、`SQLITE_BUSY` のリトライ。

use std::time::{Duration, Instant};

use rusqlite::{
    params, Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior,
};

use super::db;
use crate::index::SearchError;
use crate::normalize::NORMALIZE_VERSION;
use crate::tokenize::Tokenizer;
use crate::vector::Embedder;

pub(super) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SETUP_RETRY_INTERVAL: Duration = Duration::from_millis(20);
/// FTS5 の設定。変更したら版を上げる（meta で照合される）。
const FTS_SCHEMA: &str = "unicode61-keep-diacritics-v1";

/// 初期化中のエラー。`SQLITE_BUSY` だけをリトライ対象として見分けるために rusqlite のエラーを残す。
pub(super) enum SetupError {
    Sqlite(rusqlite::Error),
    Search(SearchError),
}

impl From<rusqlite::Error> for SetupError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<SearchError> for SetupError {
    fn from(e: SearchError) -> Self {
        Self::Search(e)
    }
}

impl From<SetupError> for SearchError {
    fn from(e: SetupError) -> Self {
        match e {
            SetupError::Sqlite(e) => db(e),
            SetupError::Search(e) => e,
        }
    }
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// meta の値を照合する。未登録の場合、空の索引（`may_register`）なら登録し、
/// 文書が入っているのに欠けているなら拒否する（古い規則で作られた索引を現行扱いにしない）。
fn ensure_meta(
    tx: &Transaction<'_>,
    key: &str,
    value: &str,
    what: &str,
    may_register: bool,
) -> Result<(), SetupError> {
    let stored: Option<String> = tx
        .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
        .optional()?;
    match stored {
        Some(v) if v == value => Ok(()),
        Some(v) => Err(SearchError::Config(format!(
            "{what}が索引と一致しません: 保存済み {v} / 指定 {value}（再索引が必要）"
        ))
        .into()),
        None if may_register => {
            tx.execute(
                "INSERT INTO meta(key, value) VALUES(?1, ?2)",
                params![key, value],
            )?;
            Ok(())
        }
        None => Err(SearchError::Config(format!(
            "索引のメタ情報（{what}）が欠けています。再索引が必要です"
        ))
        .into()),
    }
}

pub(super) fn enable_wal(conn: &Connection) -> Result<(), SetupError> {
    let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
    if mode.eq_ignore_ascii_case("wal") {
        Ok(())
    } else {
        Err(SearchError::Storage(format!(
            "WAL を有効にできません（journal_mode={mode}）。この保存先は対応していない可能性があります"
        ))
        .into())
    }
}

fn create_schema(
    conn: &mut Connection,
    tokenizer: &dyn Tokenizer,
    embedder: &dyn Embedder,
) -> Result<(), SetupError> {
    let dim = embedder.dim();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS docs(rowid INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE);
         CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
             tokens, tokenize = \"unicode61 remove_diacritics 0 tokenchars '-_'\");",
    )?;
    let empty = !tx.query_row("SELECT EXISTS(SELECT 1 FROM docs)", [], |r| {
        r.get::<_, bool>(0)
    })?;
    ensure_meta(&tx, "dim", &dim.to_string(), "埋め込み次元", empty)?;
    ensure_meta(&tx, "tokenizer_id", &tokenizer.id(), "トークナイザ", empty)?;
    ensure_meta(&tx, "embedder_id", &embedder.id(), "埋め込みモデル", empty)?;
    ensure_meta(
        &tx,
        "normalize_version",
        &NORMALIZE_VERSION.to_string(),
        "正規化の版",
        empty,
    )?;
    ensure_meta(&tx, "fts_schema", FTS_SCHEMA, "全文検索の設定", empty)?;
    // dim は usize 由来なので SQL へ埋め込んでも注入の余地はない。
    tx.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS docs_vec USING vec0(
             embedding float[{dim}] distance_metric=cosine);"
    ))?;
    tx.commit()?;
    Ok(())
}

/// 接続設定とスキーマ作成。別接続との競合で BUSY になりうる（特に新規ファイルの WAL 切替）。
/// `busy_timeout` は 1 回の試行でロックを待つ最大時間。
fn setup(
    conn: &mut Connection,
    tokenizer: &dyn Tokenizer,
    embedder: &dyn Embedder,
    persistent: bool,
    busy_timeout: Duration,
) -> Result<(), SetupError> {
    conn.busy_timeout(busy_timeout)?;
    // 索引ファイルが差し替えられても、スキーマ内の関数・ビューを信頼しない。
    conn.pragma_update(None, "trusted_schema", "OFF")?;
    if persistent {
        enable_wal(conn)?;
    }
    create_schema(conn, tokenizer, embedder)
}

/// 差し替え・破損した索引ファイルを、スキーマ作成や検索の前に弾く。
/// 大きな DB では時間がかかるので、BUSY のリトライの外で 1 回だけ実行する。
fn quick_check(conn: &Connection, timeout: Duration) -> Result<(), SetupError> {
    conn.busy_timeout(timeout)?;
    // 検査の前から、スキーマ内の関数・ビューを信頼しない。
    conn.pragma_update(None, "trusted_schema", "OFF")?;
    let verdict: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
    if verdict != "ok" {
        return Err(
            SearchError::Storage(format!("索引ファイルが破損しています: {verdict}")).into(),
        );
    }
    Ok(())
}

/// 永続 DB は、最初に `quick_check` を 1 回だけ実行する（ロック待ちは `timeout` まで）。
/// そのあと `setup` を BUSY のあいだ、`timeout` を上限にリトライする。
/// 各試行のロック待ちも残り時間に収めるので、リトライ部分は `timeout` を大きく超えない。
pub(super) fn setup_with_retry(
    conn: &mut Connection,
    tokenizer: &dyn Tokenizer,
    embedder: &dyn Embedder,
    persistent: bool,
    timeout: Duration,
) -> Result<(), SearchError> {
    if persistent {
        quick_check(conn, timeout).map_err(|e| match e {
            SetupError::Sqlite(e) if is_busy(&e) => SearchError::Storage(format!(
                "索引の検査がロック競合で {timeout:?} 以内に完了しませんでした: {e}"
            )),
            other => other.into(),
        })?;
    }
    // 検査にかかった時間は、スキーマ作成のリトライの予算に含めない（それぞれ `timeout` まで）。
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let attempt = setup(
            conn,
            tokenizer,
            embedder,
            persistent,
            remaining.max(Duration::from_millis(1)),
        );
        match attempt {
            Ok(()) => return Ok(()),
            Err(SetupError::Sqlite(e)) if is_busy(&e) => {
                if Instant::now() >= deadline {
                    return Err(SearchError::Storage(format!(
                        "索引の初期化がロック競合で {timeout:?} 以内に完了しませんでした: {e}"
                    )));
                }
                std::thread::sleep(SETUP_RETRY_INTERVAL.min(remaining));
            }
            Err(e) => return Err(e.into()),
        }
    }
}
