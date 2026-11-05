//! 文書の分割。見出しと段落で区切り、元の位置（行番号）を保ったまま、検索の単位（チャンク）にする。
//!
//! 分割は決定的（同じ文書なら同じチャンク列）。チャンクの位置は文書を分割し直せば復元できるので、
//! 位置の情報を別に保存しない。分割の結果が変わる変更をしたら [`CHUNKER_VERSION`] を上げ、索引を作り直す。
//!
//! Markdown の扱い（CommonMark に近い範囲）:
//! - 見出しは `#`〜`######` の後に空白が続く行（インデントは 3 つまで）。閉じの `#` は、直前が空白のときだけ外す
//!   （`# C#` の題名は `C#`）。見出しだけで本文が無い節はチャンクを作らない。
//! - コードブロックは、同じ記号で長さが開始以上のフェンスで閉じる。閉じていなければ、末尾までがコードブロック。
//!   コードブロックの中は見出しにも空行の区切りにもしない。

use crate::error::WorkspaceError;

/// 1 つのチャンクの目標の長さ（文字数）。短い段落はここまで束ねる。
pub const TARGET_CHARS: usize = 400;
/// 1 つのチャンクの最大の長さ（文字数）。1 つの段落がこれを超えるときは、文の切れ目で分ける。
pub const MAX_CHARS: usize = 800;
/// 見出し 1 つの最大文字数。超える分は切る（見出しは全チャンクに複製されるので、増幅を防ぐ）。
pub const MAX_HEADING_CHARS: usize = 100;
/// 1 文書の最大サイズ。
pub const MAX_DOC_BYTES: usize = 256 * 1024;
/// 1 文書の最大チャンク数。
pub const MAX_CHUNKS: usize = 5000;
/// 分割方式の版。
pub const CHUNKER_VERSION: u32 = 2;

/// 文書の形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// 段落（空行）だけで区切る。
    Text,
    /// 見出し（`#`）と段落で区切る。コードブロック（```）の中は区切らない。
    Markdown,
}

/// 検索の単位。行番号は 1 始まりで、`end_line` を含む。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub seq: usize,
    /// 所属する見出しの並び（外側から内側へ）。
    pub heading_path: Vec<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
}

impl Chunk {
    /// 索引に入れる文字列。見出しも含めて、見出しの語でも探せるようにする。
    pub fn indexed_text(&self) -> String {
        if self.heading_path.is_empty() {
            self.text.clone()
        } else {
            format!("{}\n{}", self.heading_path.join(" > "), self.text)
        }
    }
}

/// 段落（空行で区切られた連続行）。
struct Para {
    start: usize,
    end: usize,
    text: String,
}

/// 開いているコードブロック。
struct Fence {
    marker: char,
    len: usize,
}

/// 文書を分割する。空の文書は空の列になる。
pub fn split(text: &str, format: Format) -> Result<Vec<Chunk>, WorkspaceError> {
    if text.len() > MAX_DOC_BYTES {
        return Err(WorkspaceError::TooLarge {
            size: text.len(),
            max: MAX_DOC_BYTES,
        });
    }
    if text.contains('\0') {
        return Err(WorkspaceError::InvalidText("NUL 文字"));
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text); // 先頭の BOM（行番号は変わらない）
    let mut out: Vec<Chunk> = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut section: Vec<Para> = Vec::new();
    let mut cur: Option<Para> = None;
    let mut fence: Option<Fence> = None;

    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if format == Format::Markdown {
            if let Some(open) = &fence {
                if closes(line, open) {
                    fence = None;
                }
                push_line(&mut cur, n, line);
                continue;
            }
            if let Some(open) = opens(line) {
                fence = Some(open);
                push_line(&mut cur, n, line);
                continue;
            }
            if let Some((level, title)) = heading(line) {
                flush_para(&mut cur, &mut section);
                pack(std::mem::take(&mut section), &names(&path), &mut out)?;
                path.retain(|(l, _)| *l < level);
                path.push((level, title));
                continue;
            }
        }
        if line.trim().is_empty() {
            flush_para(&mut cur, &mut section);
        } else {
            push_line(&mut cur, n, line);
        }
    }
    flush_para(&mut cur, &mut section);
    pack(section, &names(&path), &mut out)?;
    for (seq, chunk) in out.iter_mut().enumerate() {
        chunk.seq = seq;
    }
    Ok(out)
}

fn names(path: &[(usize, String)]) -> Vec<String> {
    path.iter().map(|(_, t)| t.clone()).collect()
}

fn push_line(cur: &mut Option<Para>, n: usize, line: &str) {
    match cur {
        Some(p) => {
            p.text.push('\n');
            p.text.push_str(line);
            p.end = n;
        }
        None => {
            *cur = Some(Para {
                start: n,
                end: n,
                text: line.to_string(),
            })
        }
    }
}

fn flush_para(cur: &mut Option<Para>, section: &mut Vec<Para>) {
    if let Some(p) = cur.take() {
        section.push(p);
    }
}

/// 行頭のインデントが 3 つ以内なら、インデントを除いた残り。
fn unindented(line: &str) -> Option<&str> {
    let t = line.trim_start_matches(' ');
    (line.len() - t.len() <= 3 && !t.starts_with('\t')).then_some(t)
}

/// コードブロックの開始行（`` ``` `` か `~~~` が 3 つ以上）。バッククォートの開始行は、情報文字列に
/// `` ` `` を含められない（`` ```x``` `` のような行内コードは開始ではない）。
fn opens(line: &str) -> Option<Fence> {
    let t = unindented(line)?;
    let marker = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = t.chars().take_while(|&c| c == marker).count();
    let info = &t[len..];
    (len >= 3 && !(marker == '`' && info.contains('`'))).then_some(Fence { marker, len })
}

/// コードブロックの終了行: 同じ記号が開始以上の長さで続き、あとは空白だけ。
fn closes(line: &str, open: &Fence) -> bool {
    let Some(t) = unindented(line) else {
        return false;
    };
    let len = t.chars().take_while(|&c| c == open.marker).count();
    len >= open.len && t[len..].trim().is_empty()
}

/// `# 見出し` の行なら（階層, 題名）。インデントが 4 つ以上の行（コード）と、題名が空の行は見出しにしない。
fn heading(line: &str) -> Option<(usize, String)> {
    let t = unindented(line)?;
    let level = t.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &t[level..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let body = rest.trim();
    // 閉じの `#` は、直前が空白のときだけ外す（`# C#` の `#` は題名の一部）。
    let stripped = body.trim_end_matches('#');
    let title = if stripped.len() == body.len()
        || (!stripped.is_empty() && !stripped.ends_with([' ', '\t']))
    {
        body
    } else {
        stripped.trim_end()
    };
    let title: String = title.chars().take(MAX_HEADING_CHARS).collect();
    (!title.is_empty()).then_some((level, title))
}

/// 1 つの節（見出しの下の段落の列）を、目標の長さまで束ねてチャンクにする。
fn pack(paras: Vec<Para>, path: &[String], out: &mut Vec<Chunk>) -> Result<(), WorkspaceError> {
    let mut acc: Option<Para> = None;
    let emit = |p: Para, out: &mut Vec<Chunk>| -> Result<(), WorkspaceError> {
        if out.len() >= MAX_CHUNKS {
            return Err(WorkspaceError::TooManyChunks { max: MAX_CHUNKS });
        }
        out.push(Chunk {
            seq: 0,
            heading_path: path.to_vec(),
            start_line: p.start,
            end_line: p.end,
            text: p.text,
        });
        Ok(())
    };
    for p in paras.into_iter().flat_map(split_long) {
        acc = match acc {
            None => Some(p),
            Some(mut a) => {
                if a.text.chars().count() + 2 + p.text.chars().count() <= TARGET_CHARS {
                    a.text.push_str("\n\n");
                    a.text.push_str(&p.text);
                    a.end = p.end;
                    Some(a)
                } else {
                    emit(a, out)?;
                    Some(p)
                }
            }
        };
    }
    if let Some(a) = acc {
        emit(a, out)?;
    }
    Ok(())
}

/// 長すぎる段落を、文の切れ目（。！？．!? と改行）で分ける。切れ目が無いときは文字数で切る。
/// 行番号は、段落の先頭からの改行を 1 度だけ数えて求める（計算量は段落の長さに比例）。
fn split_long(p: Para) -> Vec<Para> {
    if p.text.chars().count() <= MAX_CHARS {
        return vec![p];
    }
    let mut pieces: Vec<Para> = Vec::new();
    let (mut start, mut count) = (0usize, 0usize);
    let (mut counted_to, mut newlines) = (0usize, 0usize);
    let mut flush = |from: usize, to: usize, pieces: &mut Vec<Para>| {
        let raw = &p.text[from..to];
        let lead = raw.len() - raw.trim_start_matches(['\n', '\r']).len();
        let body = raw.trim_matches(['\n', '\r']).trim_end();
        if body.trim().is_empty() {
            return;
        }
        newlines += p.text[counted_to..from + lead].matches('\n').count();
        counted_to = from + lead;
        let first = p.start + newlines;
        pieces.push(Para {
            start: first,
            end: first + body.matches('\n').count(),
            text: body.to_string(),
        });
    };
    let mut last_break = 0usize;
    for (off, c) in p.text.char_indices() {
        count += 1;
        let end = off + c.len_utf8();
        if matches!(c, '。' | '．' | '！' | '？' | '!' | '?' | '\n') {
            last_break = end;
        }
        if count >= TARGET_CHARS && last_break > start {
            flush(start, last_break, &mut pieces);
            start = last_break;
            count = p.text[start..end].chars().count();
        } else if count >= MAX_CHARS {
            flush(start, end, &mut pieces);
            (start, last_break, count) = (end, end, 0);
        }
    }
    flush(start, p.text.len(), &mut pieces);
    pieces
}

#[cfg(test)]
mod tests;
