//! Wikipedia（日本語版）の検索 API の応答を、出典の一覧にする。通信は呼び出し側が行い、ここは応答の JSON を読むだけ。
//!
//! 想定する応答は、`action=query&generator=search&prop=extracts|info&inprop=url&exintro=1&explaintext=1&formatversion=2`。
//! 応答は信用せず、URL は日本語版 Wikipedia のものだけを受け付ける。

use serde_json::Value;

use super::{
    squash, truncate, WebSource, MAX_SOURCES, MAX_SOURCE_TEXT_CHARS, MAX_SOURCE_TITLE_CHARS,
};

/// 出典として受け付ける URL の先頭。
pub const URL_PREFIX: &str = "https://ja.wikipedia.org/";
/// 解析前の応答の最大バイト数。
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// 応答から出典を取り出す（検索順。本文が空・URL が不正なものは捨てる）。読めない応答は空にする。
pub fn parse_sources(json: &str) -> Vec<WebSource> {
    if json.len() > MAX_RESPONSE_BYTES {
        return Vec::new();
    }
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(pages) = root.pointer("/query/pages").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut ranked: Vec<(u64, WebSource)> = pages
        .iter()
        .filter_map(|p| {
            let url = p.get("fullurl")?.as_str()?;
            if !url.starts_with(URL_PREFIX) || url.chars().any(|c| c.is_control() || c == ' ') {
                return None;
            }
            let text = squash(p.get("extract")?.as_str()?);
            if text.is_empty() {
                return None;
            }
            let rank = p.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX);
            Some((
                rank,
                WebSource {
                    title: truncate(&squash(p.get("title")?.as_str()?), MAX_SOURCE_TITLE_CHARS),
                    url: url.to_string(),
                    text: truncate(&text, MAX_SOURCE_TEXT_CHARS),
                },
            ))
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked
        .into_iter()
        .take(MAX_SOURCES)
        .map(|(_, s)| s)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_returned_in_search_order_and_cleaned() {
        let json = r#"{"query":{"pages":[
          {"title":"B","index":2,"extract":"二番目\n本文","fullurl":"https://ja.wikipedia.org/wiki/B"},
          {"title":"A","index":1,"extract":"一番目","fullurl":"https://ja.wikipedia.org/wiki/A"}]}}"#;
        let s = parse_sources(json);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].title, "A");
        assert_eq!(s[1].text, "二番目 本文");
    }

    #[test]
    fn foreign_urls_and_empty_extracts_are_dropped() {
        let json = r#"{"query":{"pages":[
          {"title":"X","index":1,"extract":"本文","fullurl":"https://evil.example/wiki/X"},
          {"title":"Y","index":2,"extract":"  ","fullurl":"https://ja.wikipedia.org/wiki/Y"},
          {"title":"Z","index":3,"extract":"本文","fullurl":"http://ja.wikipedia.org/wiki/Z"},
          {"title":"W","index":4,"extract":"本文","fullurl":"https://ja.wikipedia.org.evil.example/"},
          {"title":"V","index":5,"extract":"本文","fullurl":"https://ja.wikipedia.org/wiki/V"}]}}"#;
        let s = parse_sources(json);
        // `ja.wikipedia.org.evil.example` は先頭が `https://ja.wikipedia.org.` で、`/` までは一致しない
        assert_eq!(
            s.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(),
            ["V"]
        );
    }

    #[test]
    fn unreadable_or_oversized_responses_give_nothing() {
        assert!(parse_sources("not json").is_empty());
        assert!(parse_sources(r#"{"query":{}}"#).is_empty());
        assert!(parse_sources(&" ".repeat(MAX_RESPONSE_BYTES + 1)).is_empty());
    }

    #[test]
    fn count_and_length_are_capped() {
        let page = |i: usize| {
            format!(
                r#"{{"title":"T{i}","index":{i},"extract":"{}","fullurl":"https://ja.wikipedia.org/wiki/T{i}"}}"#,
                "あ".repeat(2000)
            )
        };
        let pages: Vec<String> = (1..=9).map(page).collect();
        let json = format!(r#"{{"query":{{"pages":[{}]}}}}"#, pages.join(","));
        let s = parse_sources(&json);
        assert_eq!(s.len(), MAX_SOURCES);
        assert!(s
            .iter()
            .all(|x| x.text.chars().count() == MAX_SOURCE_TEXT_CHARS));
    }
}
