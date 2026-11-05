//! 書き出しで使う日付・時刻の整形（外部の日付ライブラリに頼らない、必要最小限の暦計算）。
//!
//! 録音の時刻は UNIX ミリ秒（UTC）で持ち、表示するときだけ、呼び出し側が渡す UTC からの差（分）で
//! 現地の時刻に直す。

/// 暦の日付（年・月・日）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Date {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

/// 現地の日付と時刻。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DateTime {
    pub date: Date,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// 1970-01-01 からの日数 → 暦の日付（グレゴリオ暦。H. Hinnant の方式）。
fn civil_from_days(days: i64) -> Date {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    Date { year, month, day }
}

/// 暦の日付 → 1970-01-01 からの日数。
pub(super) fn days_from_civil(date: Date) -> i64 {
    let y = date.year - i64::from(date.month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((date.month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(date.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// UNIX ミリ秒を、UTC からの差 `offset_minutes` の現地の日時にする。
pub(super) fn local(ms: u64, offset_minutes: i32) -> DateTime {
    let secs = (ms / 1000) as i64 + i64::from(offset_minutes) * 60;
    let days = secs.div_euclid(86_400);
    let in_day = secs.rem_euclid(86_400) as u32;
    DateTime {
        date: civil_from_days(days),
        hour: in_day / 3600,
        minute: in_day % 3600 / 60,
        second: in_day % 60,
    }
}

/// `2026-11-05 13:55` の形。
pub(super) fn format_local(ms: u64, offset_minutes: i32) -> String {
    let t = local(ms, offset_minutes);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        t.date.year, t.date.month, t.date.day, t.hour, t.minute
    )
}

/// `+09:00` の形（UTC からの差）。
pub(super) fn format_offset(offset_minutes: i32) -> String {
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let m = offset_minutes.unsigned_abs();
    format!("{sign}{:02}:{:02}", m / 60, m % 60)
}

/// iCalendar の UTC 日時 `20261105T135507Z`。
pub(super) fn ics_utc(ms: u64) -> String {
    let t = local(ms, 0);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        t.date.year, t.date.month, t.date.day, t.hour, t.minute, t.second
    )
}

/// iCalendar の日付 `20261112`。
pub(super) fn ics_date(d: Date) -> String {
    format!("{:04}{:02}{:02}", d.year, d.month, d.day)
}

/// 録音の開始からの経過時間（ミリ秒）を `mm:ss`（1 時間以上は `h:mm:ss`）にする。
pub(super) fn clock(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, sec) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m:02}:{sec:02}")
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

/// 期限として受け付ける年の範囲（書き出す時刻の範囲と同じ）。
const YEARS: std::ops::RangeInclusive<i64> = 2024..=2200;

fn valid(year: i64, month: u32, day: u32) -> Option<Date> {
    let ok = YEARS.contains(&year)
        && (1..=12).contains(&month)
        && day >= 1
        && day <= days_in_month(year, month);
    ok.then_some(Date { year, month, day })
}

/// 数字だけの文字列（符号・空白を許さない）を数にする。桁数は `digits` の範囲。
fn number(s: &str, digits: std::ops::RangeInclusive<usize>) -> Option<u32> {
    (digits.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// 期限の自由記述から、日付として読めるものだけを取り出す。読めなければ `None`（無理に推測しない）。
/// 読める形: `2026-11-12`、`2026/11/12`、`2026年11月12日`、`11月12日`（年は録音の日付から決める。
/// 録音の日付より前になるときは翌年）。年は 4 桁の数字で 2024〜2200、符号は受け付けない。
/// 「来週金曜」のような相対表現は読まない。
pub(super) fn parse_due(text: &str, recorded: Date) -> Option<Date> {
    let t: String = text.trim().chars().map(fullwidth_to_ascii).collect();
    // 2026-11-12 / 2026/11/12（区切りは同じものを 2 回）
    for sep in ['-', '/'] {
        let parts: Vec<&str> = t.split(sep).collect();
        if let [y, m, d] = parts[..] {
            return valid(
                i64::from(number(y, 4..=4)?),
                number(m, 1..=2)?,
                number(d, 1..=2)?,
            );
        }
    }
    // 2026年11月12日
    if let Some((y, rest)) = t.split_once('年') {
        let (m, rest) = rest.split_once('月')?;
        let d = rest.strip_suffix('日')?;
        return valid(
            i64::from(number(y.trim(), 4..=4)?),
            number(m.trim(), 1..=2)?,
            number(d.trim(), 1..=2)?,
        );
    }
    // 11月12日（年は録音の日付から）
    let (m, rest) = t.split_once('月')?;
    let d = rest.strip_suffix('日')?;
    let (m, d) = (number(m.trim(), 1..=2)?, number(d.trim(), 1..=2)?);
    let this_year = valid(recorded.year, m, d)?;
    if days_from_civil(this_year) >= days_from_civil(recorded) {
        Some(this_year)
    } else {
        valid(recorded.year + 1, m, d)
    }
}

fn fullwidth_to_ascii(c: char) -> char {
    match c {
        '０'..='９' => char::from(b'0' + (c as u32 - '０' as u32) as u8),
        '－' | 'ー' | '‐' => '-',
        '／' => '/',
        _ => c,
    }
}
