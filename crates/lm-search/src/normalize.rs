//! 表記ゆれの正規化: 全角半角、半角カナ、濁点の合成、ハイフン類、旧字体、漢数字、語末長音、大文字小文字。
//!
//! 索引側・クエリ側に同じ関数を適用する。挙動を変えたら `NORMALIZE_VERSION` を上げること
//! （SQLite 索引は版が違うと開けない。索引は原文を持たないので、再索引は原本からやり直す）。

mod numerals;
#[cfg(test)]
mod tests;

use numerals::fold_kanji_numbers;

/// 正規化規則の版。SQLite 索引の meta に保存し、不一致なら再索引を促す。
pub const NORMALIZE_VERSION: u32 = 5;

/// 旧字体 → 新字体（頻出のみ。必要に応じて拡張する）。
const OLD_TO_NEW: &[(char, char)] = &[
    ('學', '学'),
    ('國', '国'),
    ('條', '条'),
    ('會', '会'),
    ('體', '体'),
    ('號', '号'),
    ('點', '点'),
    ('戰', '戦'),
    ('實', '実'),
    ('發', '発'),
    ('變', '変'),
    ('廣', '広'),
    ('澤', '沢'),
    ('邊', '辺'),
    ('櫻', '桜'),
];

const HALF_KANA: &str = "ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン";
const HALF_KANA_START: u32 = 0xFF66;
const HALF_KANA_COUNT: u32 = 56;

/// 濁点を付けられる清音（小書きの「っ/ッ」は含めない）。
const DAKUTEN_BASES: &str =
    "かきくけこさしすせそたちつてとはひふへほカキクケコサシスセソタチツテトハヒフヘホ";
const HANDAKUTEN_BASES: &str = "はひふへほハヒフヘホ";

fn is_katakana(c: char) -> bool {
    ('\u{30A1}'..='\u{30FA}').contains(&c) || c == '\u{30FC}'
}

/// 濁点・半濁点（半角 FF9E/FF9F、結合 3099/309A）。
fn is_voicing_mark(c: char) -> bool {
    matches!(c, '\u{FF9E}' | '\u{FF9F}' | '\u{3099}' | '\u{309A}')
}

fn half_kana_to_full(c: char) -> Option<char> {
    let code = c as u32;
    if !(HALF_KANA_START..HALF_KANA_START + HALF_KANA_COUNT).contains(&code) {
        return None;
    }
    HALF_KANA.chars().nth((code - HALF_KANA_START) as usize)
}

/// 清音に濁点・半濁点を合成する。できなければ None。
fn voice(base: char, mark: char) -> Option<char> {
    match mark {
        '\u{FF9E}' | '\u{3099}' => match base {
            'う' => Some('ゔ'),
            'ウ' => Some('ヴ'),
            b if DAKUTEN_BASES.contains(b) => char::from_u32(b as u32 + 1),
            _ => None,
        },
        '\u{FF9F}' | '\u{309A}' if HANDAKUTEN_BASES.contains(base) => {
            char::from_u32(base as u32 + 2)
        }
        _ => None,
    }
}

/// 1 文字の幅・記号の統一。`prev` は直前に出力した文字。
fn map_char(c: char, prev: Option<char>) -> char {
    match c {
        '\u{3000}' => ' ',
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        // ハイフン類（‐ ‑ ‒ − ﹣）は型番の区切りとして '-' にそろえる。
        // en dash（U+2013）は範囲表記（2020–2024年）の区切りなので畳まない。
        '\u{2010}'..='\u{2012}' | '\u{2212}' | '\u{FE63}' => '-',
        '\u{FF70}' => 'ー',
        // 「―」は直前がカタカナのときだけ長音とみなす（「東京―大阪」のダッシュは保つ）。
        '\u{2015}' if prev.is_some_and(is_katakana) => 'ー',
        _ => half_kana_to_full(c).unwrap_or(c),
    }
}

/// 全角 ASCII → 半角、半角カナ → 全角カナ、濁点・半濁点の合成（NFD 入力を含む）。
/// 合成できなかった孤立の濁点・半濁点は意味を持たないので捨てる。
fn fold_width(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let mapped = map_char(chars[i], out.chars().next_back());
        if let Some(v) = chars.get(i + 1).and_then(|&m| voice(mapped, m)) {
            out.push(v);
            i += 2;
        } else {
            if !is_voicing_mark(mapped) {
                out.push(mapped);
            }
            i += 1;
        }
    }
    out
}

fn fold_old_kanji(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            OLD_TO_NEW
                .iter()
                .find(|(old, _)| *old == c)
                .map_or(c, |(_, new)| *new)
        })
        .collect()
}

/// 語末の長音「ー」を落とす（サーバー/サーバ、コンピューター/コンピュータ）。
///
/// 「バー」「カー」のような 2 文字語は別語（バ、カ）と衝突するので落とさない
/// （カタカナ連続が 3 文字以上のときのみ）。トレードオフとして、複合語内の語末長音も落ちる
/// ため（ゴルフバー→ゴルフバ）、部分語「バー」では「ゴルフバー」を引けない。
pub fn drop_trailing_long_vowel(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == 'ー' && i > 0 && is_katakana(chars[i - 1]) {
            let next_kata = chars.get(i + 1).is_some_and(|&n| is_katakana(n));
            if !next_kata {
                let mut start = i;
                while start > 0 && is_katakana(chars[start - 1]) {
                    start -= 1;
                }
                if i - start + 1 >= 3 {
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// 幅・濁点・ハイフン・旧字体・漢数字・大文字小文字だけを揃える（語末の長音は残す）。
/// 形態素解析の前処理用。解析後のトークンには [`drop_trailing_long_vowel`] を適用する。
pub fn fold(input: &str) -> String {
    fold_kanji_numbers(&fold_old_kanji(&fold_width(input))).to_lowercase()
}

/// 検索用の正規化。索引側・クエリ側の両方に同じ関数を適用する。
pub fn normalize(input: &str) -> String {
    drop_trailing_long_vowel(&fold(input))
}
