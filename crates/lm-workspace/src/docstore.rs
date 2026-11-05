//! 文書の原本の保存。1 文書 1 ファイル（`<id>.doc`）。置き換えは一時ファイル経由で原子的に行う。
//!
//! ファイルの形式（テキスト）: 1 行目 `LMDOC1`、2 行目 `format=text|markdown`、3 行目 `title=<題名>`、
//! 4 行目は空行、5 行目以降が本文。
//!
//! 読み込み時の「確認してから開く」の間に、アプリ自身が別のファイルへ差し替える競合は防がない
//! （保存先はアプリ専用で、書き込めるのはアプリだけという前提）。

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::chunker::{Format, MAX_DOC_BYTES};
use crate::error::WorkspaceError;

const MAGIC: &str = "LMDOC1";
const EXT: &str = "doc";
const VERSION_FILE: &str = "chunker.version";
/// 題名の最大文字数。
pub const MAX_TITLE_CHARS: usize = 200;
/// ヘッダー（形式・題名）に見込む最大バイト数。本文と合わせた読み込みの上限に使う。
const MAX_HEADER_BYTES: usize = 4 * MAX_TITLE_CHARS + 64;
/// 古い一時ファイルとみなす最小の経過時間（書き込み中のものを消さないため、これより短くはできない）。
pub const MIN_TMP_AGE: Duration = Duration::from_secs(60);
/// Windows の予約デバイス名（拡張子が付いても、デバイスとして扱われる）。
const RESERVED: [&str; 4] = ["con", "prn", "aux", "nul"];

/// 保存した文書。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDoc {
    pub id: String,
    pub title: String,
    pub format: Format,
    pub text: String,
}

/// 文書の id: 小文字の英数字・ハイフン・アンダースコアの 1〜64 文字。
/// パスの区切りや `~` は使えない。大文字は使えない（大文字小文字を区別しない OS で別の文書が同じファイルに
/// なるのを防ぐ）。`con`・`nul`・`com1`・`lpt1` などの OS の予約名も使えない。
pub fn check_id(id: &str) -> Result<(), WorkspaceError> {
    let ok = (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && !is_reserved(id);
    if ok {
        Ok(())
    } else {
        Err(WorkspaceError::InvalidId)
    }
}

fn is_reserved(id: &str) -> bool {
    let numbered = |prefix: &str| {
        id.strip_prefix(prefix)
            .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
    };
    RESERVED.contains(&id) || numbered("com") || numbered("lpt")
}

/// 画面の見かけを偽装できる文字（双方向制御・ゼロ幅・行区切りなど）か。
fn is_invisible_format(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

pub fn check_title(title: &str) -> Result<(), WorkspaceError> {
    if title.trim().is_empty() {
        return Err(WorkspaceError::InvalidTitle("空です"));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(WorkspaceError::InvalidTitle("長すぎます"));
    }
    if title
        .chars()
        .any(|c| c.is_control() || is_invisible_format(c))
    {
        return Err(WorkspaceError::InvalidTitle(
            "改行・制御文字・見えない文字は使えません",
        ));
    }
    Ok(())
}

/// 入出力のエラーを、パスを含まない短い文言にする（原因の種類だけ残す）。
fn io_message(e: &std::io::Error, what: &'static str) -> WorkspaceError {
    WorkspaceError::Io(match e.kind() {
        ErrorKind::PermissionDenied => "権限がありません",
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => "保存領域が足りません",
        _ => what,
    })
}

pub struct DocStore {
    dir: PathBuf,
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

impl DocStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.{EXT}"))
    }

    /// 保存する（同じ id があれば置き換える）。
    pub fn save(&self, doc: &StoredDoc) -> Result<(), WorkspaceError> {
        check_id(&doc.id)?;
        check_title(&doc.title)?;
        if doc.text.len() > MAX_DOC_BYTES {
            return Err(WorkspaceError::TooLarge {
                size: doc.text.len(),
                max: MAX_DOC_BYTES,
            });
        }
        fs::create_dir_all(&self.dir).map_err(|e| io_message(&e, "保存先を作れません"))?;
        let format = match doc.format {
            Format::Text => "text",
            Format::Markdown => "markdown",
        };
        let body = format!(
            "{MAGIC}\nformat={format}\ntitle={}\n\n{}",
            doc.title, doc.text
        );
        let tmp = self.dir.join(format!(
            "{}.{}-{}.tmp",
            doc.id,
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let write = || -> std::io::Result<()> {
            let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            f.write_all(body.as_bytes())?;
            f.sync_all()?;
            drop(f);
            rename_with_retry(&tmp, &self.path(&doc.id))
        };
        write().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io_message(&e, "書き込めません")
        })
    }

    /// 読み込む。無ければ `NotFound`。通常のファイル以外（リンクなど）と、形式が違うものは `Corrupt`。
    pub fn load(&self, id: &str) -> Result<StoredDoc, WorkspaceError> {
        check_id(id)?;
        let path = self.path(id);
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(WorkspaceError::NotFound),
            Err(e) => return Err(io_message(&e, "読み込めません")),
        };
        if !meta.is_file() {
            return Err(WorkspaceError::Corrupt);
        }
        let limit = (MAX_DOC_BYTES + MAX_HEADER_BYTES) as u64;
        let mut raw = String::new();
        File::open(&path)
            .and_then(|f| f.take(limit + 1).read_to_string(&mut raw))
            .map_err(|_| WorkspaceError::Corrupt)?;
        if raw.len() as u64 > limit {
            return Err(WorkspaceError::Corrupt);
        }
        parse(id, &raw)
    }

    /// 削除する。無ければ `false`。
    pub fn remove(&self, id: &str) -> Result<bool, WorkspaceError> {
        check_id(id)?;
        match fs::remove_file(self.path(id)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io_message(&e, "削除できません")),
        }
    }

    /// 保存してある文書の id（昇順）。形式が合わないファイル名は無視する。
    pub fn ids(&self) -> Result<Vec<String>, WorkspaceError> {
        Ok(self.entries()?.into_iter().map(|(id, _)| id).collect())
    }

    /// 文書の件数と、原本の合計バイト数（容量の上限の判定に使う）。
    pub fn stats(&self) -> Result<(usize, u64), WorkspaceError> {
        let entries = self.entries()?;
        Ok((entries.len(), entries.iter().map(|(_, size)| size).sum()))
    }

    /// 文書の id とファイルサイズ（id 昇順）。リンクやディレクトリは含めない。
    fn entries(&self) -> Result<Vec<(String, u64)>, WorkspaceError> {
        let dir = match fs::read_dir(&self.dir) {
            Ok(d) => d,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_message(&e, "一覧を読めません")),
        };
        let suffix = format!(".{EXT}");
        let mut out: Vec<(String, u64)> = dir
            .filter_map(Result::ok)
            .filter_map(|e| {
                let meta = e
                    .metadata()
                    .ok()
                    .filter(|m| m.is_file() && !m.file_type().is_symlink())?;
                let name = e.file_name().into_string().ok()?;
                let id = name.strip_suffix(&suffix)?;
                check_id(id).ok().map(|_| (id.to_string(), meta.len()))
            })
            .collect();
        out.sort();
        Ok(out)
    }

    /// 文書の原本のサイズ（バイト）。無ければ `None`。
    pub fn byte_len(&self, id: &str) -> Option<u64> {
        check_id(id).ok()?;
        let meta = fs::symlink_metadata(self.path(id)).ok()?;
        meta.is_file().then_some(meta.len())
    }

    /// 書き込み途中で残った一時ファイル（`<id>.<pid>-<連番>.tmp`）を、`min_age` 以上古いものだけ消す。
    /// `min_age` は [`MIN_TMP_AGE`] より短くはできない（書き込み中のファイルを消さないため）。消した数を返す。
    pub fn clean_stale_tmp(&self, min_age: Duration) -> usize {
        let min_age = min_age.max(MIN_TMP_AGE);
        let Ok(dir) = fs::read_dir(&self.dir) else {
            return 0;
        };
        let now = SystemTime::now();
        dir.filter_map(Result::ok)
            .filter(|e| is_tmp_name(&e.file_name().to_string_lossy()))
            .filter(|e| {
                e.metadata()
                    .ok()
                    .filter(|m| m.is_file() && !m.file_type().is_symlink())
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| now.duration_since(t).ok())
                    .is_some_and(|age| age >= min_age)
            })
            .filter(|e| fs::remove_file(e.path()).is_ok())
            .count()
    }

    /// 索引を作ったときの分割方式の版。無い・読めないときは `None`。
    pub fn read_version(&self) -> Option<u32> {
        fs::read_to_string(self.dir.join(VERSION_FILE))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    pub fn write_version(&self, version: u32) -> Result<(), WorkspaceError> {
        fs::create_dir_all(&self.dir).map_err(|e| io_message(&e, "保存先を作れません"))?;
        fs::write(self.dir.join(VERSION_FILE), version.to_string())
            .map_err(|e| io_message(&e, "書き込めません"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// `<id>.<pid>-<連番>.tmp` の形か。
fn is_tmp_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".tmp") else {
        return false;
    };
    let Some((id, tag)) = stem.rsplit_once('.') else {
        return false;
    };
    let Some((pid, seq)) = tag.split_once('-') else {
        return false;
    };
    check_id(id).is_ok()
        && !pid.is_empty()
        && !seq.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && seq.bytes().all(|b| b.is_ascii_digit())
}

/// 置き換え先を別の処理が開いていると、Windows では一時的に失敗しうるので、短く待って数回試す。
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..4 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(20 * (attempt + 1)));
    }
    Err(last.expect("1 回以上は試している"))
}

fn parse(id: &str, raw: &str) -> Result<StoredDoc, WorkspaceError> {
    let mut parts = raw.splitn(5, '\n');
    let (magic, format, title, blank, text) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next().unwrap_or(""),
    );
    if magic != Some(MAGIC) || blank != Some("") {
        return Err(WorkspaceError::Corrupt);
    }
    let format = match format.and_then(|f| f.strip_prefix("format=")) {
        Some("text") => Format::Text,
        Some("markdown") => Format::Markdown,
        _ => return Err(WorkspaceError::Corrupt),
    };
    let title = title
        .and_then(|t| t.strip_prefix("title="))
        .ok_or(WorkspaceError::Corrupt)?;
    check_title(title).map_err(|_| WorkspaceError::Corrupt)?;
    Ok(StoredDoc {
        id: id.to_string(),
        title: title.to_string(),
        format,
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests;
