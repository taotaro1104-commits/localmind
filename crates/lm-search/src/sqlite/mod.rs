//! SQLite 永続索引: FTS5（BM25）+ sqlite-vec（cosine KNN）。feature `sqlite` で有効。
//!
//! - 全文検索は `Tokenizer` が出したトークンを空白区切りで FTS5 に格納する
//!   （FTS5 側は `unicode61` で空白分割のみ。分音記号は Rust 側（インメモリ版）と同じく区別する）。
//! - ベクトルは `vec0` 仮想表（`distance_metric=cosine`）。使えない埋め込み（ゼロ・非有限）の
//!   文書は vec0 に載せず、FTS のみで検索される。
//! - `docs.rowid` を FTS5・vec0 の rowid として共有する。
//! - meta に次元・tokenizer・embedder・正規化版・FTS 設定を記録し、不一致の索引は開けない
//!   （静かな品質劣化を防ぐ）。文書が入っているのに meta が欠けた DB も開けない。
//!   索引は原文を保持しないので、再索引は原本からやり直す必要がある。
//! - 初期化（WAL 切替・スキーマ作成）は、別接続との競合（`SQLITE_BUSY`）を一定時間リトライする。
//! - 入力の上限は `index::{MAX_QUERY_CHARS, MAX_QUERY_TERMS, MAX_DOC_BYTES, MAX_K}`。

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::index::{
    limit_query, query_terms, signals, DocSignals, LexicalIndex, SearchError, SemanticIndex,
    MAX_DOC_BYTES, MAX_DOC_ID_BYTES, MAX_EMBEDDING_DIM, MAX_K,
};
use crate::tokenize::Tokenizer;
use crate::vector::{check_len, is_usable, Embedder};
use crate::Hit;

mod dbfile;
mod setup;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_embedding;
#[cfg(test)]
mod tests_persist;
#[cfg(test)]
mod tests_regression;
#[cfg(test)]
mod tests_setup;

use dbfile::{create_private_db_file, db_path};
use setup::{setup_with_retry, BUSY_TIMEOUT};

/// sqlite-vec を以後に開く全接続へ自動登録した結果（プロセスで 1 回）。0 なら成功。
static REGISTER_VEC: OnceLock<i32> = OnceLock::new();

type VecInit = unsafe extern "C" fn(
    *mut rusqlite::ffi::sqlite3,
    *mut *mut std::os::raw::c_char,
    *const rusqlite::ffi::sqlite3_api_routines,
) -> std::os::raw::c_int;

fn register_sqlite_vec() -> Result<(), SearchError> {
    let rc = *REGISTER_VEC.get_or_init(|| {
        // SAFETY: sqlite-vec 公式の登録方法（同クレートのテストと同じ）。`sqlite3_vec_init` は
        // 拡張初期化関数で、SQLite が接続作成時に上記シグネチャで呼び出す。
        unsafe {
            let init = std::mem::transmute::<*const (), VecInit>(
                sqlite_vec::sqlite3_vec_init as *const (),
            );
            rusqlite::ffi::sqlite3_auto_extension(Some(init))
        }
    });
    if rc == rusqlite::ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(SearchError::Storage(format!(
            "sqlite-vec の登録に失敗しました (rc={rc})"
        )))
    }
}

fn db(e: rusqlite::Error) -> SearchError {
    SearchError::Storage(e.to_string())
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// 1 語を FTS5 のフレーズとして引用する（演算子・列フィルタとして解釈させない）。
fn quote_term(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// FTS5 が 1 語として扱える文字（英数字）を含むか。含まない語は MATCH に渡さない。
fn is_matchable(term: &str) -> bool {
    term.chars().any(char::is_alphanumeric)
}

/// FTS5 の MATCH 式を安全に組み立てる: 各語を引用符で囲み OR で結ぶ。
fn match_expr(terms: &[String]) -> Option<String> {
    let quoted: Vec<String> = terms
        .iter()
        .filter(|t| is_matchable(t))
        .map(|t| quote_term(t))
        .collect();
    (!quoted.is_empty()).then(|| quoted.join(" OR "))
}

/// 書き込みトランザクション（開始時に書き込みロックを取る。読み取りからの昇格で BUSY にならない）。
fn begin_write(conn: &mut Connection) -> Result<Transaction<'_>, SearchError> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db)
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
    tokenizer: Box<dyn Tokenizer>,
    embedder: Box<dyn Embedder>,
}

impl SqliteStore {
    pub fn in_memory(
        tokenizer: Box<dyn Tokenizer>,
        embedder: Box<dyn Embedder>,
    ) -> Result<Self, SearchError> {
        register_sqlite_vec()?;
        Self::init(
            Connection::open_in_memory().map_err(db)?,
            tokenizer,
            embedder,
            false,
        )
    }

    pub fn open(
        path: impl AsRef<Path>,
        tokenizer: Box<dyn Tokenizer>,
        embedder: Box<dyn Embedder>,
    ) -> Result<Self, SearchError> {
        register_sqlite_vec()?;
        // バンドル版の SQLite は URI 解釈が既定で有効（フラグから外しても無効にならない）ので、
        // パスを絶対パスにして `file:` で始まらないようにし、ただのファイル名として開く。
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let path = db_path(path.as_ref())?;
        create_private_db_file(&path)?;
        let conn = Connection::open_with_flags(&path, flags).map_err(db)?;
        Self::init(conn, tokenizer, embedder, true)
    }

    fn init(
        mut conn: Connection,
        tokenizer: Box<dyn Tokenizer>,
        embedder: Box<dyn Embedder>,
        persistent: bool,
    ) -> Result<Self, SearchError> {
        if embedder.dim() == 0 || embedder.dim() > MAX_EMBEDDING_DIM {
            return Err(SearchError::Config(format!(
                "埋め込み次元が 0、または上限 {MAX_EMBEDDING_DIM} を超えています: {}",
                embedder.dim()
            )));
        }
        setup_with_retry(
            &mut conn,
            tokenizer.as_ref(),
            embedder.as_ref(),
            persistent,
            BUSY_TIMEOUT,
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            tokenizer,
            embedder,
        })
    }

    /// 接続ロック。中毒していても復旧する（ロック内は rusqlite 呼び出しのみで、
    /// トランザクションは Drop でロールバックされるため状態は壊れない）。
    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 文書を追加、または同じ id なら内容を置き換える。
    /// `MAX_DOC_BYTES` を超える文書は拒否する（呼び出し側で分割して登録する）。
    pub fn upsert(&self, id: &str, text: &str) -> Result<(), SearchError> {
        if id.is_empty() || id.len() > MAX_DOC_ID_BYTES {
            return Err(SearchError::Config(format!(
                "文書 id が空、または {MAX_DOC_ID_BYTES} バイトを超えています"
            )));
        }
        if text.len() > MAX_DOC_BYTES {
            return Err(SearchError::Config(format!(
                "文書が大きすぎます: {} バイト（上限 {MAX_DOC_BYTES}）。分割して登録してください",
                text.len()
            )));
        }
        let tokens = self.tokenizer.tokenize(text).join(" ");
        let embedding = self.embedder.embed_document(text)?;
        check_len(self.embedder.as_ref(), &embedding)?;

        let mut conn = self.lock();
        let tx = begin_write(&mut conn)?;
        let rowid = match find_rowid(&tx, id)? {
            Some(rowid) => {
                delete_index_rows(&tx, rowid)?;
                rowid
            }
            None => {
                tx.execute("INSERT INTO docs(id) VALUES(?1)", [id])
                    .map_err(db)?;
                tx.last_insert_rowid()
            }
        };
        tx.execute(
            "INSERT INTO docs_fts(rowid, tokens) VALUES(?1, ?2)",
            params![rowid, tokens],
        )
        .map_err(db)?;
        if is_usable(&embedding) {
            tx.execute(
                "INSERT INTO docs_vec(rowid, embedding) VALUES(?1, ?2)",
                params![rowid, to_blob(&embedding)],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)
    }

    /// 文書を削除する。存在すれば true。
    pub fn remove(&self, id: &str) -> Result<bool, SearchError> {
        let mut conn = self.lock();
        let tx = begin_write(&mut conn)?;
        let Some(rowid) = find_rowid(&tx, id)? else {
            return Ok(false);
        };
        delete_index_rows(&tx, rowid)?;
        tx.execute("DELETE FROM docs WHERE rowid=?1", [rowid])
            .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(true)
    }

    pub fn len(&self) -> Result<usize, SearchError> {
        let n: i64 = self
            .lock()
            .query_row("SELECT count(*) FROM docs", [], |r| r.get(0))
            .map_err(db)?;
        Ok(n as usize)
    }

    pub fn is_empty(&self) -> Result<bool, SearchError> {
        Ok(self.len()? == 0)
    }

    /// `prefix` で始まる文書 id（昇順）。空の接頭辞なら全件。孤児になった項目の掃除に使う。
    /// 接頭辞は文字単位で比べる（`LIKE` のワイルドカードとして解釈しない）。
    pub fn ids_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SearchError> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT id FROM docs WHERE substr(id, 1, ?1) = ?2 ORDER BY id")
            .map_err(db)?;
        let rows = stmt
            .query_map(params![prefix.chars().count() as i64, prefix], |r| r.get(0))
            .map_err(db)?;
        rows.collect::<Result<Vec<String>, _>>().map_err(db)
    }

    /// 現在の journal_mode（テスト用）。
    #[cfg(test)]
    fn journal_mode(&self) -> String {
        self.lock()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap_or_default()
    }

    /// 現在の trusted_schema（0 = 信頼しない。テスト用）。
    #[cfg(test)]
    fn trusted_schema(&self) -> i64 {
        self.lock()
            .query_row("PRAGMA trusted_schema", [], |r| r.get(0))
            .unwrap_or(-1)
    }
}

fn find_rowid(tx: &Transaction<'_>, id: &str) -> Result<Option<i64>, SearchError> {
    tx.query_row("SELECT rowid FROM docs WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(db)
}

fn delete_index_rows(tx: &Transaction<'_>, rowid: i64) -> Result<(), SearchError> {
    tx.execute("DELETE FROM docs_fts WHERE rowid=?1", [rowid])
        .map_err(db)?;
    tx.execute("DELETE FROM docs_vec WHERE rowid=?1", [rowid])
        .map_err(db)?;
    Ok(())
}

impl LexicalIndex for SqliteStore {
    fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>, SearchError> {
        let terms = query_terms(self.tokenizer.as_ref(), query);
        let (Some(expr), true) = (match_expr(&terms), k > 0) else {
            return Ok(Vec::new());
        };
        let limit = k.min(MAX_K) as i64;
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT d.id, bm25(docs_fts) AS s FROM docs_fts
                 JOIN docs d ON d.rowid = docs_fts.rowid
                 WHERE docs_fts MATCH ?1 ORDER BY s, d.id LIMIT ?2",
            )
            .map_err(db)?;
        let rows = stmt
            .query_map(params![expr, limit], |r| {
                Ok(Hit {
                    id: r.get(0)?,
                    score: -(r.get::<_, f64>(1)? as f32), // FTS5 の bm25 は小さいほど良い
                })
            })
            .map_err(db)?;
        let mut hits = Vec::new();
        for row in rows {
            let hit = row.map_err(db)?;
            if hit.score.is_finite() && hit.score > 0.0 {
                hits.push(hit);
            }
        }
        Ok(hits)
    }

    /// クエリ語（最大 `MAX_QUERY_TERMS`）ごとに「その文書に含まれるか」だけを FTS5 に問い合わせる。
    /// 文書全体のトークンを読み込まないので、巨大文書がヒットしてもメモリを食わない。
    fn doc_signals(&self, id: &str, query: &str) -> Result<Option<DocSignals>, SearchError> {
        let terms = query_terms(self.tokenizer.as_ref(), query);
        let mut conn = self.lock();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(db)?;
        let Some(rowid) = find_rowid(&tx, id)? else {
            return Ok(None);
        };
        let mut stmt = tx
            .prepare("SELECT 1 FROM docs_fts WHERE rowid=?1 AND docs_fts MATCH ?2")
            .map_err(db)?;
        let mut present = HashSet::new();
        for term in terms.iter().filter(|t| is_matchable(t)) {
            if stmt.exists(params![rowid, quote_term(term)]).map_err(db)? {
                present.insert(term.clone());
            }
        }
        Ok(Some(signals(&terms, |t| present.contains(t))))
    }
}

impl SemanticIndex for SqliteStore {
    fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>, SearchError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let limit = k.min(MAX_K) as i64;
        let q = self.embedder.embed_query(limit_query(query))?;
        check_len(self.embedder.as_ref(), &q)?;
        if !is_usable(&q) {
            return Ok(Vec::new()); // ゼロ・非有限ベクトルは cosine が定義できない
        }
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT d.id, v.distance FROM
                   (SELECT rowid, distance FROM docs_vec WHERE embedding MATCH ?1 AND k = ?2) v
                 JOIN docs d ON d.rowid = v.rowid ORDER BY v.distance, d.id",
            )
            .map_err(db)?;
        let rows = stmt
            .query_map(params![to_blob(&q), limit], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<f64>>(1)?))
            })
            .map_err(db)?;
        let mut hits = Vec::new();
        for row in rows {
            let (id, distance) = row.map_err(db)?;
            // 距離が NULL/非有限の行は検索不能な文書なので捨てる（検索全体は失敗させない）。
            let Some(d) = distance.filter(|d| d.is_finite()) else {
                continue;
            };
            let score = 1.0 - d as f32; // cosine 距離 → 類似度
            if score > 0.0 {
                hits.push(Hit { id, score });
            }
        }
        Ok(hits)
    }
}
