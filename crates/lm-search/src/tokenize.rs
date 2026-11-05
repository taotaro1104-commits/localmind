//! トークナイザ。Lindera 等の形態素解析器は `Tokenizer` trait の実装として差し替える。

use crate::normalize::normalize;

pub trait Tokenizer: Send + Sync {
    /// 正規化済みトークン列を返す（重複を保持、出現順）。
    fn tokenize(&self, text: &str) -> Vec<String>;
    /// 索引との整合確認に使う識別子。トークンの切り方が変わったら必ず変える。
    fn id(&self) -> String;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    /// 漢字・かな（bi-gram 化）
    Cjk,
    /// 英数字（単語として保持）
    Word,
    Other,
}

fn classify(c: char) -> Class {
    match c {
        // 中黒（ジョン・スミス）は区切り。bi-gram に混ぜない。
        '\u{30FB}' => Class::Other,
        // 々・〆 は漢字と同じ連続として扱う（is_alphanumeric では Word になってしまう）。
        '\u{3005}' | '\u{3006}' => Class::Cjk,
        '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' => Class::Cjk,
        _ if c.is_alphanumeric() => Class::Word,
        _ => Class::Other,
    }
}

/// 正規化済みテキストを分割した連続部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// 漢字・かなの連続（形態素解析や bi-gram の対象）
    Cjk(String),
    /// 英数字語（型番など。そのまま 1 トークン）
    Word(String),
}

fn push_cjk(run: &mut Vec<char>, out: &mut Vec<Run>) {
    if !run.is_empty() {
        out.push(Run::Cjk(run.iter().collect()));
        run.clear();
    }
}

fn push_word(word: &mut String, out: &mut Vec<Run>) {
    let trimmed = word.trim_matches(|c| c == '-' || c == '_');
    if !trimmed.is_empty() {
        out.push(Run::Word(trimmed.to_string()));
    }
    word.clear();
}

/// テキストを正規化し、CJK 連続部分と英数字語に分割する。
pub fn split_runs(text: &str) -> Vec<Run> {
    split_normalized(&normalize(text))
}

/// 正規化済みの文字列を分割する。正規化方法を呼び出し側が選びたい場合に使う。
/// 型番の連結子: 英数字に挟まれた '-' '_' は語に含める。
pub fn split_normalized(normalized: &str) -> Vec<Run> {
    let chars: Vec<char> = normalized.chars().collect();
    let mut out = Vec::new();
    let mut cjk: Vec<char> = Vec::new();
    let mut word = String::new();

    for (i, &c) in chars.iter().enumerate() {
        match classify(c) {
            Class::Cjk => {
                push_word(&mut word, &mut out);
                cjk.push(c);
            }
            Class::Word => {
                push_cjk(&mut cjk, &mut out);
                word.push(c);
            }
            Class::Other => {
                push_cjk(&mut cjk, &mut out);
                let joins = (c == '-' || c == '_')
                    && !word.is_empty()
                    && chars
                        .get(i + 1)
                        .is_some_and(|&n| classify(n) == Class::Word);
                if joins {
                    word.push(c);
                } else {
                    push_word(&mut word, &mut out);
                }
            }
        }
    }
    push_cjk(&mut cjk, &mut out);
    push_word(&mut word, &mut out);
    out
}

/// 文字 bi-gram（CJK）＋英数字語。辞書不要で、型番・条文番号・人名が外れにくい。
#[derive(Debug, Default, Clone, Copy)]
pub struct NgramTokenizer;

impl Tokenizer for NgramTokenizer {
    fn id(&self) -> String {
        "ngram-v1".into()
    }

    fn tokenize(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for run in split_runs(text) {
            match run {
                Run::Word(w) => out.push(w),
                Run::Cjk(c) => {
                    let chars: Vec<char> = c.chars().collect();
                    if chars.len() == 1 {
                        out.push(c);
                    } else {
                        out.extend(chars.windows(2).map(|w| w.iter().collect::<String>()));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str) -> Vec<String> {
        NgramTokenizer.tokenize(s)
    }

    #[test]
    fn japanese_becomes_bigrams() {
        assert_eq!(tok("契約更新"), vec!["契約", "約更", "更新"]);
    }

    #[test]
    fn single_cjk_char_is_kept() {
        assert_eq!(tok("条"), vec!["条"]);
    }

    #[test]
    fn model_number_is_one_token() {
        assert_eq!(tok("型番 AB-1234 です"), vec!["型番", "ab-1234", "です"]);
    }

    #[test]
    fn full_width_model_number_matches_half_width() {
        assert_eq!(tok("ＡＢ－１２３４"), tok("ab-1234"));
    }

    #[test]
    fn article_number_splits_on_script_boundaries() {
        assert_eq!(tok("第12条"), vec!["第", "12", "条"]);
    }

    #[test]
    fn trailing_hyphen_and_punctuation_are_dropped() {
        assert_eq!(tok("abc- def."), vec!["abc", "def"]);
    }

    #[test]
    fn split_runs_separates_scripts() {
        assert_eq!(
            split_runs("型番 AB-1234 です"),
            vec![
                Run::Cjk("型番".into()),
                Run::Word("ab-1234".into()),
                Run::Cjk("です".into())
            ]
        );
    }

    #[test]
    fn iteration_mark_stays_inside_the_cjk_run() {
        assert_eq!(tok("人々"), vec!["人々"]);
        let t = tok("様々な人々");
        assert!(!t.contains(&"々".to_string()), "{t:?}");
        assert_eq!(t, vec!["様々", "々な", "な人", "人々"]);
    }

    #[test]
    fn middle_dot_separates_words() {
        assert_eq!(tok("ジョン・スミス"), vec!["ジョ", "ョン", "スミ", "ミス"]);
    }

    #[test]
    fn nfd_and_nfc_dakuten_give_the_same_tokens() {
        assert_eq!(tok("か\u{3099}く"), tok("がく"));
    }

    #[test]
    fn kanji_and_arabic_article_numbers_tokenize_alike() {
        assert_eq!(tok("第十二条"), tok("第12条"));
    }

    #[test]
    fn empty_input_gives_no_tokens() {
        assert!(tok("").is_empty());
        assert!(tok("  ,。").is_empty());
    }
}
