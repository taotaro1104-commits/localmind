//! 漢数字 → 算用数字。
//!
//! 目的は「第十二条」と「第12条」、「令和六年四月一日」と「令和6年4月1日」を同じ表記にすること。
//! 日常語（一度・一人・十分・一日・八百屋・七五三）を壊さないよう、変換は次の文脈に限る:
//!   1. 「第」の直後（ただし「第三者」「第一人者」は除く）
//!   2. 直後が法令・暦の語（条項章節編款号月）
//!   3. 2 文字以上の漢数字列の直後が一般の助数詞（日年時分秒円人回個件番階歳点本枚台名度倍割位期）
//!   4. 十・百・千の単独の直後が数量の助数詞（十円・百人。「十分」は含めない）
//!   5. 暦・時刻の文脈: 「月」の直後の「日」、元号の直後の「年」、「時」「時間」の直後の「分」、
//!      「分」の直後の「秒」、単独の数字＋「時」（「一時停止」「一時的」は除く。
//!      「午前」「午後」の直後・直後が「間」「半」の「一時」は時刻として変換する）
//!   6. 二〜九の単独の直後が人回日年個件階歳本枚台名度倍割円点番期（一と十は日常語と衝突するため
//!      対象外。「五分五分」「三日月」「二日酔い」「四日市」も除く）
//!   7. 元号の「元年」は「1年」にそろえる（令和元年 = 令和1年）
//!
//! 「次第」「落第」「及第」の「第」は序数の前置きではないので、直後の漢数字は変換しない。
//! 一致しない表記（許容）: 二分の一などの分数、万・億を含む金額、一丁目などの未登録の助数詞。
//!
//! 「四半期」は「第一四半期」を「第14半期」にしないよう、数字列の末尾の「四」を切り離す。
//! 「万」「億」に隣接する列は未対応のため変換しない（「三万五千円」はそのまま）。

const NUMERALS: &str = "〇一二三四五六七八九十百千";
const STRONG_COUNTERS: &str = "条項章節編款号月";
const GENERAL_COUNTERS: &str = "日年時分秒円人回個件番階歳点本枚台名度倍割位期";
const UNIT_COUNTERS: &str = "円人回個件階歳本枚台名";
/// 二〜九の単独で数量とみなす助数詞（一・十は日常語と衝突しやすいので含めない。「分」も除く）。
const SINGLE_DIGITS: &str = "二三四五六七八九";
const SINGLE_DIGIT_COUNTERS: &str = "人回日年個件階歳本枚台名度倍割円点番期";
const ERAS: [(char, char); 5] = [
    ('令', '和'),
    ('平', '成'),
    ('昭', '和'),
    ('大', '正'),
    ('明', '治'),
];
/// 桁あふれを避けるための位取り表記の最大桁数。
const MAX_POSITIONAL_DIGITS: usize = 18;

fn digit_value(c: char) -> Option<u64> {
    "〇一二三四五六七八九"
        .chars()
        .position(|d| d == c)
        .map(|p| p as u64)
}

fn unit_value(c: char) -> Option<u64> {
    match c {
        '十' => Some(10),
        '百' => Some(100),
        '千' => Some(1000),
        _ => None,
    }
}

/// 漢数字列を算用数字の文字列にする（「十二」→"12"、「二千二十五」→"2025"、「〇九〇」→"090"）。
/// 単位（千→百→十）は降順で各 1 回までとし、不正な並びは None。
fn parse_kanji_number(run: &[char]) -> Option<String> {
    if run.iter().all(|&c| digit_value(c).is_some()) {
        if run.len() > MAX_POSITIONAL_DIGITS {
            return None;
        }
        return Some(
            run.iter()
                .filter_map(|&c| digit_value(c))
                .map(|d| d.to_string())
                .collect(),
        );
    }
    let (mut total, mut pending, mut last_unit) = (0u64, None::<u64>, u64::MAX);
    for &c in run {
        if let Some(d) = digit_value(c) {
            if pending.is_some() {
                return None; // 「三五十」のような不正な並び
            }
            pending = Some(d);
        } else if let Some(u) = unit_value(c) {
            if u >= last_unit || pending == Some(0) {
                return None; // 単位は降順・各 1 回。「〇十」も不正
            }
            total += pending.take().unwrap_or(1) * u;
            last_unit = u;
        }
    }
    if last_unit != u64::MAX && pending == Some(0) {
        return None; // 「三十〇」のように単位の後に 〇 が続く並びも不正
    }
    Some((total + pending.unwrap_or(0)).to_string())
}

/// 「第三者」「第一人者」や、「次第」「落第」「及第」の「第」は序数ではないので変換しない。
fn is_ordinal_word(chars: &[char], start: usize, end: usize) -> bool {
    let compound_dai = start >= 2 && "次落及".contains(chars[start - 2]);
    compound_dai
        || match chars.get(end) {
            Some('者') => true,
            Some('人') => chars.get(end + 1) == Some(&'者'),
            _ => false,
        }
}

/// 「三日月」「二日酔い」「四日市」のような、数字＋日を含む語。
fn is_day_compound(chars: &[char], end: usize) -> bool {
    matches!(chars.get(end + 1), Some('月' | '酔' | '市'))
}

/// `start` から `back` 文字前が数字（漢数字または ASCII 数字）か。
fn digit_like_at(chars: &[char], start: usize, back: usize) -> bool {
    start >= back && {
        let c = chars[start - back];
        NUMERALS.contains(c) || c.is_ascii_digit()
    }
}

/// 直前が「午前」「午後」か。
fn follows_ampm(chars: &[char], start: usize) -> bool {
    start >= 2 && matches!((chars[start - 2], chars[start - 1]), ('午', '前' | '後'))
}

/// 暦・時刻の文脈で、単独の数字でも変換してよいか。`next` は列の直後の文字。
fn is_calendar_context(chars: &[char], start: usize, end: usize, next: char) -> bool {
    let prev = start.checked_sub(1).map(|p| chars[p]);
    match next {
        '日' => prev == Some('月'),
        '年' => start >= 2 && ERAS.contains(&(chars[start - 2], chars[start - 1])),
        // 「一時停止」「一時的」を避ける。「一時間」「一時半」「一時十五分」「午後一時」は時刻。
        '時' => {
            let after = chars.get(end + 1);
            chars[start..end] != ['一']
                || matches!(after, Some('間' | '半'))
                || after.is_some_and(|c| NUMERALS.contains(*c))
                || follows_ampm(chars, start)
        }
        // 「三時十分」「二時間五分」。数字を伴わない「時間十分」「期間十分」は語なので除く。
        '分' => {
            (prev == Some('時') && digit_like_at(chars, start, 2))
                || (prev == Some('間')
                    && start >= 2
                    && chars[start - 2] == '時'
                    && digit_like_at(chars, start, 3))
        }
        '秒' => prev == Some('分') && digit_like_at(chars, start, 2),
        _ => false,
    }
}

/// `chars[start..end]` の漢数字列を算用数字へ変換してよい文脈か。
fn should_convert(chars: &[char], start: usize, end: usize) -> bool {
    let run = &chars[start..end];
    let prev = start.checked_sub(1).map(|p| chars[p]);
    let next = chars.get(end).copied();
    if matches!(prev, Some('万' | '億')) || matches!(next, Some('万' | '億')) {
        return false;
    }
    if prev == Some('第') {
        return !is_ordinal_word(chars, start, end);
    }
    let Some(n) = next else {
        return false;
    };
    let single_digit = run.len() == 1
        && SINGLE_DIGITS.contains(run[0])
        && SINGLE_DIGIT_COUNTERS.contains(n)
        && !(n == '日' && is_day_compound(chars, end));
    STRONG_COUNTERS.contains(n)
        || (run.len() >= 2 && GENERAL_COUNTERS.contains(n))
        || (run.len() == 1 && unit_value(run[0]).is_some() && UNIT_COUNTERS.contains(n))
        || single_digit
        || is_calendar_context(chars, start, end, n)
}

/// 「令和元年」を「令和1年」にそろえる（元号の直後の「元年」だけ）。
fn fold_gannen(input: &str) -> String {
    ERAS.iter().fold(input.to_string(), |text, (a, b)| {
        text.replace(&format!("{a}{b}元年"), &format!("{a}{b}1年"))
    })
}

/// 漢数字の連続を、文脈に応じて算用数字へ置き換える。
pub(super) fn fold_kanji_numbers(input: &str) -> String {
    let input = fold_gannen(input);
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if !NUMERALS.contains(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && NUMERALS.contains(chars[i]) {
            i += 1;
        }
        let mut end = i;
        // 「四半期」の「四」は数字列に含めない（「第一四半期」→「第1四半期」）。
        if chars.get(end) == Some(&'半') && chars[end - 1] == '四' {
            end -= 1;
        }
        if end == start {
            out.push(chars[start]); // 「四」だけ（四半期）
            i = start + 1;
            continue;
        }
        i = end;
        let run = &chars[start..end];
        let converted = should_convert(&chars, start, end)
            .then(|| parse_kanji_number(run))
            .flatten();
        match converted {
            Some(n) => out.push_str(&n),
            None => out.extend(run),
        }
    }
    out
}
