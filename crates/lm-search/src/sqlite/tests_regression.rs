//! 表記ゆれ（漢数字・日付・四半期・範囲表記）の検索レベル回帰テスト。

use super::tests::store;
use super::tests_persist::hybrid;
use crate::grounding::{SearchOutcome, DEFAULT_GROUNDING_THRESHOLD};

// ---- 表記ゆれの検索レベル回帰（漢数字・日付・四半期・en dash） ---------------------------

#[test]
fn kanji_arabic_dates_quarters_and_ranges_find_each_other() {
    let s = store();
    s.upsert("q", "第一四半期の売上は好調でした").unwrap();
    s.upsert("d", "決算は令和六年四月一日から開始します")
        .unwrap();
    s.upsert("r", "2020–2024年の売上推移を示す").unwrap();
    s.upsert("t", "会議は三時十五分に始まります").unwrap();
    let h = hybrid(s, DEFAULT_GROUNDING_THRESHOLD);

    let cases = [
        ("第1四半期の売上", "q"),
        ("第一四半期の売上", "q"),
        ("四半期の売上", "q"),
        ("令和6年4月1日", "d"),
        ("4月1日", "d"),
        ("四月一日", "d"),
        ("2024年の売上推移", "r"),
        ("2020年の売上推移", "r"),
        ("3時15分", "t"),
    ];
    for (query, want) in cases {
        match h.search(query, 3).unwrap() {
            SearchOutcome::Found { hits, .. } => assert_eq!(hits[0].id, want, "{query}"),
            other => panic!("{query}: {other:?}"),
        }
    }
}

#[test]
fn everyday_kanji_numeral_words_are_still_found() {
    let s = store();
    s.upsert("a", "同時に処理できる件数の上限は100件です")
        .unwrap();
    s.upsert("b", "ひとり暮らし向けの小さな部屋の間取り")
        .unwrap();
    s.upsert("c", "充電は問題なく完了します").unwrap();
    let h = hybrid(s, DEFAULT_GROUNDING_THRESHOLD);
    for (query, want) in [
        ("一度に処理できる件数の上限", "a"),
        ("一人暮らし向けの部屋", "b"),
        ("充電は十分に完了します", "c"),
    ] {
        match h.search(query, 3).unwrap() {
            SearchOutcome::Found { hits, .. } => assert_eq!(hits[0].id, want, "{query}"),
            other => panic!("{query}: {other:?}"),
        }
    }
}
