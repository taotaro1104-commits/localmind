//! Lindera（IPADIC）による形態素解析トークナイザ。feature `lindera` で有効。
//!
//! 型番などの英数字語は `split_runs` でそのまま保持し、CJK 連続部分だけを解析する。
//! 助詞・助動詞・記号・非自立語は捨て、活用語は原形に揃える（「煮込みます」→「煮込む」）。

use std::borrow::Cow;

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;

use crate::index::SearchError;
use crate::normalize::{drop_trailing_long_vowel, fold};
use crate::tokenize::{split_normalized, Run, Tokenizer};

/// IPADIC の品詞情報（details[0]=品詞, [1]=細分類, [6]=原形）から残すか判定する。
fn keep(pos: &str, sub: &str) -> bool {
    !matches!(pos, "助詞" | "助動詞" | "記号" | "フィラー") && sub != "非自立"
}

/// 活用語は原形、それ以外は表層形を使う。
fn normal_form<'a>(pos: &str, surface: &'a str, base: Option<&'a str>) -> &'a str {
    match (pos, base) {
        ("動詞" | "形容詞", Some(b)) if b != "*" && !b.is_empty() => b,
        _ => surface,
    }
}

pub struct LinderaTokenizer {
    segmenter: Segmenter,
}

impl LinderaTokenizer {
    pub fn new() -> Result<Self, SearchError> {
        let dictionary = load_dictionary("embedded://ipadic")
            .map_err(|e| SearchError::Config(format!("Lindera 辞書の読み込みに失敗: {e}")))?;
        Ok(Self {
            segmenter: Segmenter::new(Mode::Normal, dictionary, None),
        })
    }

    fn analyze(&self, run: &str, out: &mut Vec<String>) {
        let Ok(mut tokens) = self.segmenter.segment(Cow::Borrowed(run)) else {
            // 解析失敗時は文字単位に縮退し、検索不能にはしない。
            out.extend(run.chars().map(String::from));
            return;
        };
        for token in tokens.iter_mut() {
            let surface = token.surface.to_string();
            let details: Vec<String> = token.details().iter().map(|d| d.to_string()).collect();
            let pos = details.first().map_or("", String::as_str);
            let sub = details.get(1).map_or("", String::as_str);
            if !keep(pos, sub) {
                continue;
            }
            let base = details.get(6).map(String::as_str);
            out.push(drop_trailing_long_vowel(normal_form(pos, &surface, base)));
        }
    }
}

impl Tokenizer for LinderaTokenizer {
    fn id(&self) -> String {
        "lindera-ipadic-v1".into()
    }

    fn tokenize(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        // 解析前は語末長音を残し（「カレーに」を壊さない）、トークン単位で統一する。
        for run in split_normalized(&fold(text)) {
            match run {
                Run::Word(w) => out.push(drop_trailing_long_vowel(&w)),
                Run::Cjk(c) => self.analyze(&c, &mut out),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str) -> Vec<String> {
        LinderaTokenizer::new().unwrap().tokenize(s)
    }

    #[test]
    fn keep_filters_function_words() {
        assert!(!keep("助詞", "格助詞"));
        assert!(!keep("助動詞", "*"));
        assert!(!keep("記号", "句点"));
        assert!(!keep("動詞", "非自立"));
        assert!(keep("名詞", "一般"));
        assert!(keep("接頭詞", "名詞接続"));
    }

    #[test]
    fn normal_form_uses_base_for_conjugating_words() {
        assert_eq!(normal_form("動詞", "煮込み", Some("煮込む")), "煮込む");
        assert_eq!(normal_form("名詞", "具材", Some("具材")), "具材");
        assert_eq!(normal_form("動詞", "x", Some("*")), "x");
        assert_eq!(normal_form("動詞", "x", None), "x");
    }

    #[test]
    fn drops_particles_from_a_question() {
        let t = tok("カレーに入れる具材は");
        // 解析は「カレー」のまま行い、解析後に語末長音を統一する（カレー→カレ）。
        assert!(t.contains(&"カレ".to_string()), "{t:?}");
        assert!(t.contains(&"入れる".to_string()), "{t:?}");
        assert!(
            !t.contains(&"に".to_string()) && !t.contains(&"は".to_string()),
            "{t:?}"
        );
    }

    #[test]
    fn query_and_document_forms_agree() {
        assert_eq!(tok("サーバーの移行"), tok("サーバの移行"));
    }

    #[test]
    fn conjugations_share_a_base_form() {
        assert_eq!(tok("煮込みます"), tok("煮込む"));
    }

    #[test]
    fn identifiers_pass_through_unsplit() {
        let t = tok("型番 AB-1234 の説明");
        assert!(t.contains(&"ab-1234".to_string()), "{t:?}");
    }

    #[test]
    fn article_number_still_splits_on_script_boundary() {
        let t = tok("第12条");
        assert!(t.contains(&"12".to_string()), "{t:?}");
    }

    #[test]
    fn empty_and_symbol_only_inputs() {
        assert!(tok("").is_empty());
        assert!(tok("。、").is_empty());
    }
}
