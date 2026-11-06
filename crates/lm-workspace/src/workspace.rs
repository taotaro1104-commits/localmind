//! ワークスペース本体。文書の原本（`DocStore`）を正とし、検索の索引（`SqliteStore`）はそこから作る。
//!
//! チャンクの索引上の id は `<文書 id>~<連番 5 桁>`（文書 id に `~` は使えないので、`<文書 id>~` で
//! 始まる項目はその文書のものだけ）。追加は「原本を保存 → チャンクを索引へ → 余りの項目を削除」の順。
//! 途中で索引の更新に失敗すると [`WorkspaceError::IndexOutOfDate`] を返す（原本は残る）。
//! [`Workspace::reindex`] は原本から索引を作り直し、原本に無い項目も消す。
//!
//! 追加・削除・作り直しは 1 つずつ行う（埋め込みの計算を含むので、長い文書では時間がかかる）。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use lm_search::grounding::SearchOutcome;
use lm_search::{Hit, HybridSearcher, SearchError, SqliteStore};

use crate::chunker::{split, Chunk, Format, CHUNKER_VERSION};
use crate::docstore::{check_group, check_id, check_title, DocStore, StoredDoc, MIN_TMP_AGE};
use crate::error::WorkspaceError;

/// 引用の抜粋の最大文字数。
const EXCERPT_CHARS: usize = 160;
/// 登録できる文書の最大件数。
pub const MAX_DOCS: usize = 200;
/// 原本の合計の最大バイト数。
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
/// 1 回の検索で返す最大件数（これを超える `k` は丸める）。
pub const MAX_RESULTS: usize = 50;
/// 資料ごとの足切りの幅（multilingual-e5-small 用）: 1 位との cosine の差が、これ以内の結果だけを残す。
///
/// 値はモデルごとに違う（e5 は cosine を 0.7〜0.9 に圧縮するので、小さな幅で足りる。ほかの埋め込みでは
/// 別の値が要る）。そのため足切りは既定で無効で、e5 を使うときだけ [`Workspace::with_relevance_margin`] で
/// この値を渡す。測定: サンプルの資料 3 件（7 チャンク）と質問 12 問（`examples/cutoff.rs`）で、
/// 答えの箇所は cosine 0.852 以上で 1 位（答えが 2 か所にまたがる質問は、2 位が 1 位より 0.028 低い）。
/// 答えと無関係な箇所は 1 位より 0.053 以上低かった。その間の 0.04 を採る。測った質問は少ないので、
/// 実際の資料で測り直すこと。
pub const E5_RELEVANCE_MARGIN: f32 = 0.04;

/// 登録されている文書の情報。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocInfo {
    pub id: String,
    pub title: String,
    pub format: Format,
    /// 資料のグループ名。空なら未分類。
    pub group: String,
    pub chunks: usize,
}

/// 検索結果 1 件の出典。
#[derive(Debug, Clone, PartialEq)]
pub struct Citation {
    pub doc_id: String,
    pub title: String,
    pub heading_path: Vec<String>,
    /// 原本の行番号（1 始まり、`end_line` を含む）。
    pub start_line: usize,
    pub end_line: usize,
    pub excerpt: String,
    pub score: f32,
}

/// 検索の結果。`found == false` のときは「資料には見つかりません」で、`citations` は空。
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub found: bool,
    pub grounding: f32,
    pub citations: Vec<Citation>,
}

pub struct Workspace {
    store: Arc<SqliteStore>,
    searcher: HybridSearcher,
    docs: DocStore,
    /// 追加・削除・作り直しを 1 つずつにする（原本と索引の食い違いを防ぐ）。
    write_lock: Mutex<()>,
    /// 資料ごとの足切りの幅。`f32::INFINITY` なら足切りしない。
    relevance_margin: f32,
}

fn chunk_id(doc_id: &str, seq: usize) -> String {
    format!("{doc_id}~{seq:05}")
}

fn parse_chunk_id(id: &str) -> Option<(&str, usize)> {
    let (doc, seq) = id.rsplit_once('~')?;
    Some((doc, seq.parse().ok()?))
}

fn excerpt(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut it = flat.chars();
    let head: String = it.by_ref().take(EXCERPT_CHARS).collect();
    if it.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

impl Workspace {
    pub fn new(store: Arc<SqliteStore>, searcher: HybridSearcher, docs: DocStore) -> Self {
        Self {
            store,
            searcher,
            docs,
            write_lock: Mutex::new(()),
            relevance_margin: f32::INFINITY,
        }
    }

    /// 資料ごとの足切りの幅を決める（[`E5_RELEVANCE_MARGIN`]）。既定は足切りなし（`f32::INFINITY`）。
    /// 負の値は 0（1 位と同点のものだけ）、数でない値は足切りなしに戻す。
    pub fn with_relevance_margin(mut self, margin: f32) -> Self {
        self.relevance_margin = if margin.is_nan() {
            f32::INFINITY
        } else {
            margin.max(0.0)
        };
        self
    }

    fn lock(&self) -> MutexGuard<'_, ()> {
        // 別スレッドが失敗しても、ここで止めない（ロックが守るのは順序だけで、壊れる状態は持たない）。
        self.write_lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 分割方式の版が、索引を作ったときと違えば、索引を作り直す。作り直したら `true`。
    /// 開いた直後に 1 度呼ぶ（時間がかかるので UI スレッドから呼ばないこと）。
    pub fn ensure_current(&self) -> Result<bool, WorkspaceError> {
        let seen = self.docs.read_version();
        if seen == Some(CHUNKER_VERSION) {
            return Ok(false);
        }
        // 版の記録が無く、文書も索引も空なら、作り直すものが無い（新しいワークスペース）。
        if seen.is_none() && self.docs.ids()?.is_empty() && self.store.is_empty()? {
            self.docs.write_version(CHUNKER_VERSION)?;
            return Ok(false);
        }
        self.reindex()?;
        Ok(true)
    }

    /// 文書を追加する（同じ id があれば置き換える。置き換えるときは、元のグループを引き継ぐ）。
    /// 作ったチャンクの数を返す。
    pub fn add(
        &self,
        id: &str,
        title: &str,
        format: Format,
        text: &str,
    ) -> Result<usize, WorkspaceError> {
        self.add_with(id, title, format, text, None)
    }

    /// [`Workspace::add`] に、グループを付ける。`group` が `None` なら、同じ id の文書があればそのグループを引き継ぎ、
    /// 無ければ未分類。`Some("")` は未分類にする。
    pub fn add_with(
        &self,
        id: &str,
        title: &str,
        format: Format,
        text: &str,
        group: Option<&str>,
    ) -> Result<usize, WorkspaceError> {
        check_id(id)?;
        check_title(title)?;
        let group = group.map(check_group).transpose()?;
        let chunks = split(text, format)?; // 何も書き込む前に、不正な入力を弾く
        let _guard = self.lock();
        let group = match group {
            Some(g) => g,
            None => match self.docs.load(id) {
                Ok(d) => d.group,
                // 無い・壊れている文書の置き換えは、未分類から始める。入出力の失敗は、黙って捨てずに伝える
                Err(WorkspaceError::NotFound | WorkspaceError::Corrupt) => String::new(),
                Err(e) => return Err(e),
            },
        };
        self.check_quota(id, text.len())?;
        // 版の記録は、空のワークスペースへ最初の文書を入れるときだけ書く（古い索引が残っているかもしれない
        // ワークスペースに書くと、`ensure_current` が作り直しを飛ばしてしまう）。
        let first = self.docs.read_version().is_none()
            && self.docs.ids()?.is_empty()
            && self.store.is_empty()?;
        self.docs.save(&StoredDoc {
            id: id.to_string(),
            title: title.to_string(),
            format,
            group,
            text: text.to_string(),
        })?;
        self.index_chunks(id, &chunks).map_err(stale)?;
        if first {
            self.docs
                .write_version(CHUNKER_VERSION)
                .map_err(|e| WorkspaceError::IndexOutOfDate(e.to_string()))?;
        }
        Ok(chunks.len())
    }

    /// 文書のグループを変える（空なら未分類）。検索の索引は変わらない。文書が無ければ `false`。
    pub fn set_group(&self, id: &str, group: &str) -> Result<bool, WorkspaceError> {
        check_id(id)?;
        let group = check_group(group)?;
        let _guard = self.lock();
        let mut doc = match self.docs.load(id) {
            Ok(d) => d,
            Err(WorkspaceError::NotFound) => return Ok(false),
            Err(e) => return Err(e),
        };
        doc.group = group;
        self.docs.save(&doc)?;
        Ok(true)
    }

    /// 件数と合計サイズの上限を確かめる（同じ id の置き換えは、古い版の分を差し引く）。
    fn check_quota(&self, id: &str, new_bytes: usize) -> Result<(), WorkspaceError> {
        let (count, bytes) = self.docs.stats()?;
        let old = self.docs.byte_len(id);
        let (count, bytes) = match old {
            Some(old) => (count, bytes.saturating_sub(old)),
            None => (count + 1, bytes),
        };
        if count > MAX_DOCS || bytes + new_bytes as u64 > MAX_TOTAL_BYTES {
            return Err(WorkspaceError::Full {
                max: MAX_DOCS,
                max_bytes: MAX_TOTAL_BYTES as usize,
            });
        }
        Ok(())
    }

    /// 文書のチャンクを索引へ入れ、この文書の余りの項目（以前の版の末尾など）を消す。
    fn index_chunks(&self, id: &str, chunks: &[Chunk]) -> Result<(), SearchError> {
        let mut keep = HashSet::with_capacity(chunks.len());
        for c in chunks {
            let cid = chunk_id(id, c.seq);
            self.store.upsert(&cid, &c.indexed_text())?;
            keep.insert(cid);
        }
        for stale in self.store.ids_with_prefix(&format!("{id}~"))? {
            if !keep.contains(&stale) {
                self.store.remove(&stale)?;
            }
        }
        Ok(())
    }

    /// 文書を消す。原本も索引の項目も無ければ `false`（原本が無く索引の孤児だけを消したときは `true`）。
    /// 索引を先に消し、原本は最後に消す（途中で失敗しても再試行できる）。
    /// 索引は原本の状態に頼らず、`<id>~` で始まる項目をすべて消す（原本が壊れていても孤児を残さない）。
    pub fn remove(&self, id: &str) -> Result<bool, WorkspaceError> {
        check_id(id)?;
        let _guard = self.lock();
        let ids = self.store.ids_with_prefix(&format!("{id}~"))?;
        for cid in &ids {
            self.store.remove(cid)?;
        }
        let existed = self.docs.remove(id)?;
        Ok(existed || !ids.is_empty())
    }

    /// 登録されている文書（id 順）。壊れて読めない文書は一覧に出さない。
    pub fn list(&self) -> Result<Vec<DocInfo>, WorkspaceError> {
        let mut out = Vec::new();
        for id in self.docs.ids()? {
            if let Ok(doc) = self.docs.load(&id) {
                let chunks = split(&doc.text, doc.format).map(|c| c.len()).unwrap_or(0);
                out.push(DocInfo {
                    id,
                    title: doc.title,
                    format: doc.format,
                    group: doc.group,
                    chunks,
                });
            }
        }
        Ok(out)
    }

    /// 原本から索引を作り直す（分割方式を変えたときや、索引の更新に失敗したときに使う）。
    /// 原本に無い項目（削除済みの文書、減った末尾のチャンク）も消す。読めない文書は飛ばし、その id を返す。
    /// 書き込み途中で残った古い一時ファイルも掃除する。
    pub fn reindex(&self) -> Result<Vec<String>, WorkspaceError> {
        let _guard = self.lock();
        self.docs.clean_stale_tmp(MIN_TMP_AGE);
        let mut skipped = Vec::new();
        let mut expected: HashSet<String> = HashSet::new();
        for id in self.docs.ids()? {
            let loaded = self.docs.load(&id).and_then(|d| split(&d.text, d.format));
            let Ok(chunks) = loaded else {
                skipped.push(id);
                continue;
            };
            for c in &chunks {
                let cid = chunk_id(&id, c.seq);
                self.store.upsert(&cid, &c.indexed_text()).map_err(stale)?;
                expected.insert(cid);
            }
        }
        // 掃除の対象は、この層が作った項目（`<文書 id>~<連番>`）だけ。
        for cid in self.store.ids_with_prefix("").map_err(stale)? {
            if cid.contains('~') && !expected.contains(&cid) {
                self.store.remove(&cid).map_err(stale)?;
            }
        }
        // 読めない文書があるときは版を記録しない（次に開いたときに、もう一度作り直す）。
        if skipped.is_empty() {
            self.docs.write_version(CHUNKER_VERSION)?;
        }
        Ok(skipped)
    }
}

fn stale(e: SearchError) -> WorkspaceError {
    WorkspaceError::IndexOutOfDate(e.to_string())
}

mod search;

#[cfg(test)]
mod tests;
