use super::*;

#[test]
fn full_width_ascii_becomes_half_width() {
    assert_eq!(normalize("ＡＢ－１２３４"), "ab-1234");
    assert_eq!(normalize("全角　スペース"), "全角 スペース");
}

#[test]
fn hyphen_variants_unify_for_model_numbers() {
    for dash in ['\u{2010}', '\u{2011}', '\u{2012}', '\u{2212}', '\u{FE63}'] {
        assert_eq!(
            normalize(&format!("AB{dash}1234")),
            "ab-1234",
            "U+{:04X}",
            dash as u32
        );
    }
}

#[test]
fn en_dash_keeps_numeric_ranges_separate() {
    assert_eq!(normalize("2020–2024年"), "2020–2024年");
    assert_ne!(normalize("2020–2024年"), normalize("2020-2024年"));
}

#[test]
fn half_width_katakana_becomes_full_width() {
    assert_eq!(normalize("ｶﾀｶﾅ"), "カタカナ");
}

#[test]
fn half_width_voiced_marks_are_combined() {
    assert_eq!(normalize("ｶﾞｷﾞｸﾞ"), "ガギグ");
    assert_eq!(normalize("ﾊﾟﾋﾟ"), "パピ");
    assert_eq!(normalize("ｳﾞ"), "ヴ");
    assert_eq!(normalize("ﾃﾞｰﾀ"), "データ");
}

#[test]
fn small_tsu_is_never_voiced() {
    assert_eq!(normalize("ｯﾞ"), "ッ");
}

#[test]
fn stray_voicing_marks_are_dropped() {
    assert_eq!(normalize("ｦﾞ"), "ヲ");
    assert_eq!(normalize("ﾝﾞ"), "ン");
    assert_eq!(normalize("ﾞﾞﾟ"), "");
    assert_eq!(normalize("バ\u{3099}"), "バ");
}

#[test]
fn nfd_combining_marks_are_composed() {
    assert_eq!(normalize("か\u{3099}"), "が");
    assert_eq!(normalize("ハ\u{309A}"), "パ");
    assert_eq!(normalize("が"), normalize("か\u{3099}"));
}

#[test]
fn old_kanji_maps_to_new() {
    assert_eq!(normalize("學校"), "学校");
    assert_eq!(normalize("第十二條"), "第12条");
}

#[test]
fn kanji_numerals_become_arabic_in_legal_and_calendar_contexts() {
    assert_eq!(normalize("第十二条"), "第12条");
    assert_eq!(normalize("第十三条"), "第13条");
    assert_eq!(normalize("第一条"), "第1条");
    assert_eq!(normalize("三月三十一日"), "3月31日");
    assert_eq!(normalize("十二月"), "12月");
    assert_eq!(normalize("二千二十五年"), "2025年");
    assert_eq!(normalize("二〇二五年"), "2025年");
    assert_eq!(normalize("百五号"), "105号");
    assert_eq!(normalize("〇九〇号"), "090号");
    assert_eq!(normalize("二十人"), "20人");
    assert_eq!(normalize("十円"), "10円");
    assert_eq!(normalize("百人"), "100人");
}

/// 「第」の直後は、助数詞が法令・暦の語でなくても序数として変換する。
#[test]
fn numerals_after_dai_are_ordinals() {
    assert_eq!(normalize("第一回"), "第1回");
    assert_eq!(normalize("第二次"), "第2次");
    assert_eq!(normalize("第三位"), "第3位");
}

#[test]
fn dai_words_that_are_not_ordinals_are_kept() {
    assert_eq!(normalize("第三者"), "第三者");
    assert_eq!(normalize("第一人者"), "第一人者");
    // 序数としての「第一人」は変換する。
    assert_eq!(normalize("第一人目"), "第1人目");
}

#[test]
fn quarter_is_not_swallowed_into_the_numeral_run() {
    assert_eq!(normalize("第一四半期"), "第1四半期");
    assert_eq!(normalize("第二四半期"), "第2四半期");
    assert_eq!(normalize("第1四半期"), normalize("第一四半期"));
    assert_eq!(normalize("四半期"), "四半期");
    assert_eq!(normalize("二四半期"), "二四半期");
}

#[test]
fn dates_and_times_are_converted_consistently() {
    assert_eq!(normalize("令和六年四月一日"), "令和6年4月1日");
    assert_eq!(normalize("令和6年4月1日"), "令和6年4月1日");
    assert_eq!(normalize("十月十日"), "10月10日");
    assert_eq!(normalize("四月一日"), "4月1日");
    assert_eq!(normalize("二〇二四年十二月三十一日"), "2024年12月31日");
    assert_eq!(normalize("三時十五分"), "3時15分");
    assert_eq!(normalize("三時十分"), "3時10分");
    assert_eq!(normalize("二時間三十分"), "2時間30分");
    assert_eq!(normalize("一時間"), "1時間");
    assert_eq!(normalize("九時五分三十秒"), "9時5分30秒");
    assert_eq!(normalize("平成元年"), "平成1年");
}

#[test]
fn kanji_and_arabic_dates_normalize_to_the_same_text() {
    assert_eq!(normalize("令和六年四月一日"), normalize("令和6年4月1日"));
    assert_eq!(normalize("三時十五分"), normalize("3時15分"));
    assert_eq!(normalize("十月十日"), normalize("10月10日"));
}

#[test]
fn calendar_rules_do_not_touch_everyday_words() {
    for w in [
        "一日中",
        "一日",
        "一時停止",
        "一時的",
        "一時",
        "三日月",
        "五分五分",
        "十分",
    ] {
        assert_eq!(normalize(w), w, "{w}");
    }
}

#[test]
fn era_first_year_matches_year_one() {
    assert_eq!(normalize("令和元年"), "令和1年");
    assert_eq!(normalize("令和元年"), normalize("令和1年"));
    assert_eq!(normalize("昭和元年十二月"), "昭和1年12月");
    // 元号の直後でない「元年」は触らない。
    assert_eq!(normalize("改革元年"), "改革元年");
}

#[test]
fn ampm_and_half_hour_times_match_arabic_ones() {
    assert_eq!(normalize("午後一時"), "午後1時");
    assert_eq!(normalize("午前九時"), "午前9時");
    assert_eq!(normalize("一時半"), "1時半");
    assert_eq!(normalize("一時十五分"), "1時15分");
    assert_eq!(normalize("午後一時"), normalize("午後1時"));
    // 「一時」が暫定の意味の語は変えない。
    for w in ["一時停止", "一時的", "一時預かり", "一時金"] {
        assert_eq!(normalize(w), w, "{w}");
    }
}

#[test]
fn durations_convert_minutes_and_seconds_only_in_time_context() {
    assert_eq!(normalize("二時間五分"), "2時間5分");
    assert_eq!(normalize("三時五分三十秒"), "3時5分30秒");
    assert_eq!(normalize("三時十分"), "3時10分");
    // 時刻でない「間」「分」は変えない（期間十分に…）。
    assert_eq!(normalize("期間十分"), "期間十分");
    assert_eq!(normalize("時間十分"), "時間十分");
    assert_eq!(normalize("五分五分"), "五分五分");
}

#[test]
fn single_digits_two_to_nine_convert_with_common_counters() {
    for (kanji, arabic) in [
        ("二人", "2人"),
        ("三回", "3回"),
        ("三日", "3日"),
        ("四年", "4年"),
        ("五個", "5個"),
        ("七階", "7階"),
        ("九名", "9名"),
    ] {
        assert_eq!(normalize(kanji), arabic, "{kanji}");
        assert_eq!(normalize(kanji), normalize(arabic), "{kanji}");
    }
}

#[test]
fn single_digit_rule_spares_everyday_compounds() {
    for w in [
        "三日月",
        "二日酔い",
        "四日市",
        "五分五分",
        "三つ",
        "二つ目",
        "一人",
        "一回",
        "一日",
        "九州",
        "八百屋",
    ] {
        assert_eq!(normalize(w), w, "{w}");
    }
}

#[test]
fn compound_dai_words_are_not_ordinals() {
    assert_eq!(normalize("到着次第一旦連絡"), "到着次第一旦連絡");
    assert_eq!(
        normalize("お申込み次第一括で発送"),
        "お申込み次第一括で発送"
    );
    assert_eq!(normalize("落第一回"), "落第一回");
    // 序数としての「第」は引き続き変換する。
    assert_eq!(normalize("第一回"), "第1回");
}

#[test]
fn kanji_and_arabic_article_numbers_match() {
    assert_eq!(normalize("第十二条"), normalize("第12条"));
    assert_ne!(normalize("第十二条"), normalize("第十三条"));
}

/// 日常語を壊さない（再レビューで見つかった回帰の再発防止）。
#[test]
fn everyday_words_with_numeral_chars_are_kept() {
    for w in [
        "一般",
        "十字架",
        "一緒に",
        "一度",
        "一度に",
        "一人",
        "一人暮らし",
        "十分",
        "十分に",
        "一日",
        "一番",
        "一時停止",
        "同一人物",
        "三日月",
        "八百屋",
        "七五三",
        "十二支",
        "三千世界",
        "一年",
        "一回",
    ] {
        assert_eq!(normalize(w), w, "{w}");
    }
}

#[test]
fn enough_and_ten_minutes_are_different_words() {
    assert_ne!(normalize("十分"), normalize("10分"));
    assert_eq!(normalize("充電は十分に完了"), "充電は十分に完了");
}

#[test]
fn man_and_oku_runs_are_left_alone() {
    assert_eq!(normalize("三万五千円"), "三万五千円");
    assert_eq!(normalize("二千万円"), "二千万円");
    assert_eq!(normalize("一億円"), "一億円");
}

#[test]
fn malformed_numeral_runs_are_kept() {
    for w in [
        "三五十条",
        "十十円",
        "二百千円",
        "三十四十円",
        "〇十円",
        "三十〇号",
        "二千五百〇円",
    ] {
        assert_eq!(normalize(w), w, "{w}");
    }
}

#[test]
fn huge_positional_numeral_run_does_not_overflow() {
    let run = "一".repeat(40);
    assert_eq!(normalize(&format!("{run}円")), format!("{run}円"));
}

#[test]
fn trailing_long_vowel_is_unified() {
    assert_eq!(normalize("サーバー"), normalize("サーバ"));
    assert_eq!(normalize("コンピューター"), normalize("コンピュータ"));
}

#[test]
fn short_words_keep_their_long_vowel() {
    assert_ne!(normalize("バーの予約"), normalize("バの予約"));
    assert_eq!(normalize("バー"), "バー");
    assert_eq!(normalize("カー"), "カー");
}

#[test]
fn compound_words_lose_the_trailing_long_vowel_by_design() {
    // 既知のトレードオフ: 部分語「バー」では「ゴルフバー」を引けない。
    assert_eq!(normalize("ゴルフバー"), "ゴルフバ");
}

#[test]
fn inner_long_vowel_is_kept() {
    assert!(normalize("ラーメン").contains('ー'));
}

#[test]
fn long_vowel_variants_unify() {
    assert_eq!(normalize("ラｰメン"), normalize("ラーメン"));
    assert_eq!(normalize("ラ―メン"), normalize("ラーメン"));
}

#[test]
fn em_dash_between_kanji_is_not_a_long_vowel() {
    assert_eq!(normalize("東京―大阪"), "東京―大阪");
}

#[test]
fn fold_keeps_trailing_long_vowel_but_normalizes_width() {
    assert_eq!(fold("カレーに　ＡＢ"), "カレーに ab");
    assert_eq!(drop_trailing_long_vowel("カレー"), "カレ");
    assert_eq!(drop_trailing_long_vowel("ラーメン"), "ラーメン");
}

#[test]
fn lowercases_latin() {
    assert_eq!(normalize("HELLO World"), "hello world");
}

#[test]
fn empty_stays_empty() {
    assert_eq!(normalize(""), "");
}
