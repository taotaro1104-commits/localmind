//! Web ページ（HTML）から、資料にする本文を取り出す。ネットワークには出ない（取得は呼び出し側）。
//!
//! 小さな字句解析で、タグを読み飛ばして本文だけを残す。`script` / `style` などの中身と、ナビゲーション・
//! フッターなどの本文でない部分は捨てる。見出し（`h1`〜`h6`）は Markdown の `#` に、箇条書きは `-` にして、
//! 資料の分割（見出し単位）と出典の見出しに使えるようにする。HTML は信用しない入力として扱い、
//! 出力に制御文字（改行以外）を残さない。

use crate::chunker::MAX_DOC_BYTES;
use crate::docstore::is_invisible_format;

/// 読み込む HTML の最大バイト数（これを超えた分は読まない）。
pub const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
/// 呼び出し側が本文のあとに足す行（取得元など）のために残す余地（バイト）。
const RESERVED_BYTES: usize = 4096;
/// 題名の最大文字数。
const MAX_TITLE_CHARS: usize = 100;

/// 取り出した資料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// ページの題名（`<title>`、無ければ最初の見出し）。無ければ空。
    pub title: String,
    /// Markdown に近い本文。
    pub markdown: String,
    /// 本文が大きすぎて、後ろを切ったか。
    pub truncated: bool,
}

/// 中身ごと捨てるタグ。
const SKIP: &[&str] = &[
    "noscript", "template", "svg", "iframe", "nav", "footer", "aside", "select",
];
/// 中身を文字列として読み飛ばすタグ（中の `<` や引用符をタグとして読まない）。
const RAW: &[&str] = &["script", "style"];
/// 前後で改行を入れるタグ。
const BLOCK: &[&str] = &[
    "p",
    "div",
    "br",
    "ul",
    "ol",
    "tr",
    "table",
    "section",
    "article",
    "main",
    "header",
    "blockquote",
    "pre",
    "dl",
    "dt",
    "dd",
    "figure",
    "figcaption",
    "details",
    "summary",
    "hr",
    "address",
    "body",
];

/// HTML から題名と本文を取り出す。
pub fn extract(html: &str) -> Page {
    let html = cut_at_boundary(html, MAX_HTML_BYTES);
    let (page, unclosed) = extract_with(html, true);
    if unclosed {
        // `</nav>` などの閉じ忘れで、後ろの本文まで捨ててしまった。捨てずに読み直す。
        return extract_with(html, false).0;
    }
    page
}

/// `drop_blocks` が true なら、ナビゲーションなどの本文でない部分を捨てる。戻り値の `bool` は、
/// 捨てる部分が閉じないまま終わったか。
fn extract_with(html: &str, drop_blocks: bool) -> (Page, bool) {
    let mut out = String::new();
    let mut title = String::new();
    let mut first_heading = String::new();
    let mut skip_until: Option<(String, usize)> = None; // 中身を捨てているタグ名と、同じ名前の入れ子の深さ
    let mut in_title = false;
    let mut heading: Option<usize> = None; // 見出しの中なら、その見出しの深さ
    let mut heading_text = String::new();

    let mut i = 0;
    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").map_or(rest.len(), |e| e + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            // `<!DOCTYPE ...>` や `<![CDATA[...` など。`>` までを捨てる
            i += rest.find('>').map_or(rest.len(), |e| e + 1);
            continue;
        }
        if let Some(tag) = parse_tag(rest) {
            i += tag.len;
            if let Some((skipping, depth)) = &mut skip_until {
                if &tag.name == skipping && !tag.self_closing {
                    if !tag.closing {
                        *depth += 1;
                    } else if *depth == 0 {
                        skip_until = None;
                    } else {
                        *depth -= 1;
                    }
                }
                continue;
            }
            let name = tag.name.as_str();
            if RAW.contains(&name) && !tag.closing && !tag.self_closing {
                // 閉じタグまでを、タグとして読まずに飛ばす（中の引用符や `<` に惑わされない）
                i += raw_end(&html[i..], name);
                continue;
            }
            if name == "title" {
                in_title = !tag.closing;
            } else if drop_blocks && SKIP.contains(&name) {
                if !tag.closing && !tag.self_closing {
                    skip_until = Some((tag.name, 0));
                }
            } else if name == "td" || name == "th" {
                out.push(' ');
            } else if let Some(level) = heading_level(name) {
                if tag.closing {
                    if heading.take().is_some() {
                        let t = squash(&heading_text);
                        if first_heading.is_empty() {
                            first_heading = t.clone();
                        }
                        out.push('\n');
                    }
                } else {
                    out.push_str("\n\n");
                    out.push_str(&"#".repeat(level));
                    out.push(' ');
                    heading = Some(level);
                    heading_text.clear();
                }
            } else if name == "li" && !tag.closing {
                out.push_str("\n- ");
            } else if BLOCK.contains(&name) {
                out.push('\n');
            }
            continue;
        }
        // タグではない文字（`<` が単独なら、ただの文字）
        let first = rest.chars().next().map_or(1, char::len_utf8);
        let end = rest[first..].find('<').map_or(rest.len(), |e| e + first);
        let chunk = &rest[..end];
        i += end;
        if skip_until.is_some() {
            continue;
        }
        let text = decode_entities(chunk);
        if in_title {
            title.push_str(&text);
        } else {
            if heading.is_some() {
                heading_text.push_str(&text);
            }
            out.push_str(&text);
        }
    }

    let unclosed = skip_until.is_some();
    let title = squash(if title.trim().is_empty() {
        &first_heading
    } else {
        &title
    });
    let (markdown, truncated) = tidy(&out);
    let page = Page {
        title: title.chars().take(MAX_TITLE_CHARS).collect(),
        markdown,
        truncated,
    };
    (page, unclosed)
}

/// `</name`（大文字小文字を区別しない）の `>` の次までのバイト数。無ければ残りすべて。
fn raw_end(rest: &str, name: &str) -> usize {
    let pat = format!("</{name}");
    let bytes = rest.as_bytes();
    let n = pat.len();
    let Some(at) = bytes
        .windows(n)
        .position(|w| w.eq_ignore_ascii_case(pat.as_bytes()))
    else {
        return rest.len();
    };
    rest[at..].find('>').map_or(rest.len(), |e| at + e + 1)
}

struct Tag {
    name: String,
    closing: bool,
    self_closing: bool,
    /// タグ全体のバイト数。
    len: usize,
}

/// 先頭が `<名前 ...>` / `</名前>` なら読む（属性の引用符の中の `>` は閉じとみなさない）。
fn parse_tag(s: &str) -> Option<Tag> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'<') {
        return None;
    }
    let closing = bytes.get(1) == Some(&b'/');
    let name_start = if closing { 2 } else { 1 };
    let name: String = s[name_start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if name.is_empty() || !name.as_bytes()[0].is_ascii_alphabetic() {
        return None; // `<` の直後が英字でなければ、タグではない（`<!DOCTYPE` は下で読み飛ばす）
    }
    let mut quote: Option<u8> = None;
    for (off, &b) in bytes.iter().enumerate().skip(name_start + name.len()) {
        match (quote, b) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'>') => {
                return Some(Tag {
                    name,
                    closing,
                    self_closing: bytes[off.saturating_sub(1)] == b'/',
                    len: off + 1,
                })
            }
            _ => {}
        }
    }
    // 閉じていないタグは、ここから後ろを全部タグとみなして捨てる
    Some(Tag {
        name,
        closing,
        self_closing: false,
        len: s.len(),
    })
}

fn heading_level(name: &str) -> Option<usize> {
    let rest = name.strip_prefix('h')?;
    match rest.parse::<usize>() {
        Ok(n) if (1..=6).contains(&n) && rest.len() == 1 => Some(n),
        _ => None,
    }
}

/// `max` バイト以下で、文字の途中で切らない。
fn cut_at_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        // `;` は、すぐ近く（10 バイト以内）だけ探す（長い入力を `&` ごとに端まで調べない）
        let near = tail.as_bytes().iter().take(11).position(|&b| b == b';');
        let decoded = near.and_then(|e| entity(&tail[1..e]).map(|c| (c, e + 1)));
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = &tail[used..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// 空白（改行・タブ・制御文字を含む）を 1 つの空白にする。
fn squash(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || c.is_control() || is_invisible_format(c))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 行ごとに空白を整え、空行は 1 つにまとめる。上限（[`MAX_DOC_BYTES`]）を超えたら、行の区切りで切る。
fn tidy(raw: &str) -> (String, bool) {
    let mut out = String::new();
    let mut blank = true; // 先頭の空行は入れない
    let mut truncated = false;
    for line in raw.split('\n') {
        let line = squash(line);
        if line.is_empty() {
            if !blank {
                out.push('\n');
                blank = true;
            }
            continue;
        }
        let room = (MAX_DOC_BYTES - RESERVED_BYTES).saturating_sub(out.len() + 2);
        if line.len() > room {
            truncated = true;
            // 1 行が長いだけで本文が空にならないよう、入る分だけ（文字の途中では切らない）入れる
            let mut end = room;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            if end > 0 && out.is_empty() {
                out.push_str(&line[..end]);
                out.push('\n');
            }
            break;
        }
        out.push_str(&line);
        out.push('\n');
        blank = false;
    }
    (out.trim_end().to_string() + "\n", truncated)
}

#[cfg(test)]
mod tests;
