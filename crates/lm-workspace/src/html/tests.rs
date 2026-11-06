use super::*;

#[test]
fn text_headings_and_lists_become_markdown() {
    let p = extract(
        "<!DOCTYPE html><html><head><title>出張規程 | 社内</title><style>p{color:red}</style></head>\
<body><h1>出張規程</h1><p>経費は<b>5営業日</b>以内に&amp;申請する。</p>\
<h2>宿泊</h2><ul><li>上限は1万円</li><li>領収書が必要</li></ul></body></html>",
    );
    assert_eq!(p.title, "出張規程 | 社内");
    assert_eq!(
        p.markdown,
        "# 出張規程\n\n経費は5営業日以内に&申請する。\n\n## 宿泊\n\n- 上限は1万円\n- 領収書が必要\n"
    );
    assert!(!p.truncated);
}

#[test]
fn scripts_styles_navigation_and_comments_are_dropped() {
    let p = extract(
        "<body><nav>メニュー<nav>入れ子</nav>まだメニュー</nav><script>alert('x<b>')</script>\
<!-- 隠しコメント <p>見えない</p> --><p>本文</p><footer>© 会社</footer><style>.a{}</style></body>",
    );
    assert_eq!(p.markdown, "本文\n");
}

#[test]
fn the_title_falls_back_to_the_first_heading() {
    assert_eq!(
        extract("<h2>見出しだけ</h2><p>本文</p>").title,
        "見出しだけ"
    );
    assert_eq!(extract("<p>題名なし</p>").title, "");
    let long = format!("<title>{}</title>", "あ".repeat(300));
    assert_eq!(extract(&long).title.chars().count(), MAX_TITLE_CHARS);
}

#[test]
fn entities_are_decoded_once_and_never_become_tags() {
    let p = extract("<p>&lt;script&gt;alert(1)&lt;/script&gt; &#26085;&#x672C; &nbsp;&unknown; AT&T &amp;lt;</p>");
    assert_eq!(
        p.markdown,
        "<script>alert(1)</script> 日本 &unknown; AT&T &lt;\n"
    );
}

#[test]
fn quoted_attributes_may_contain_a_greater_than_sign() {
    let p = extract(r#"<p title="a>b" data-x='<c>'>見える</p><img alt="x>y">続き"#);
    assert_eq!(p.markdown, "見える\n続き\n");
}

#[test]
fn stray_angle_brackets_and_unclosed_tags_do_not_panic_or_loop() {
    assert_eq!(extract("1 < 2 and 3 > 2").markdown, "1 < 2 and 3 > 2\n");
    assert_eq!(extract("本文<p class=\"x").markdown, "本文\n");
    for s in [
        "<", "<<<>>>", "&", "&#", "&#x;", "<!--", "<a", "</", "<p><", "<script>",
    ] {
        let _ = extract(s);
    }
    assert_eq!(extract("").markdown, "\n");
}

#[test]
fn control_characters_never_reach_the_output() {
    let p = extract("<p>a\u{0}b\u{7}c\u{202E}d\r\ne\u{b}f</p>");
    assert!(
        p.markdown.chars().all(|c| c == '\n' || !c.is_control()),
        "{:?}",
        p.markdown
    );
    assert!(!p.markdown.contains('\0'));
}

#[test]
fn big_pages_are_cut_at_a_line_and_flagged() {
    let body: String = (0..30_000)
        .map(|i| format!("<p>行{i} あいうえおかきくけこ</p>"))
        .collect();
    let p = extract(&body);
    assert!(p.truncated);
    assert!(p.markdown.len() <= MAX_DOC_BYTES);
    assert!(p.markdown.ends_with("こ\n"), "行の途中で切らない");
    // 入力そのものの上限（文字の途中で切らない）
    let huge = "あ".repeat(MAX_HTML_BYTES);
    let _ = extract(&huge);
}

#[test]
fn uppercase_tags_and_headings_work() {
    let p = extract("<HTML><BODY><H3>小見出し</H3><P>本文</P><SCRIPT>x</SCRIPT></BODY></HTML>");
    assert_eq!(p.markdown, "### 小見出し\n\n本文\n");
}

#[test]
fn a_flood_of_ampersands_is_linear_not_quadratic() {
    let start = std::time::Instant::now();
    let p = extract(&format!("<p>{}</p>", "&".repeat(MAX_HTML_BYTES - 16)));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    assert!(p.markdown.starts_with('&'));
}

#[test]
fn quotes_and_angle_brackets_inside_scripts_do_not_swallow_the_page() {
    let p = extract(
        "<script>for(i=0;i<n;i++){x='<p>'}</script><p>本文</p><STYLE>a{content:\"</p>\"}</STYLE><p>続き</p>",
    );
    assert_eq!(p.markdown, "本文\n\n続き\n");
    // 閉じていない script は、そこから後ろを捨てる（本文には混ぜない）
    assert_eq!(extract("<p>前</p><script>alert(1)").markdown, "前\n");
}

#[test]
fn an_unclosed_navigation_block_does_not_drop_the_rest_of_the_page() {
    let p = extract("<nav>メニュー<p>本文はここ</p><p>続き</p>");
    assert!(
        p.markdown.contains("本文はここ") && p.markdown.contains("続き"),
        "{:?}",
        p.markdown
    );
    let closed = extract("<nav>メニュー</nav><p>本文</p>");
    assert_eq!(
        closed.markdown, "本文\n",
        "閉じていれば、これまでどおり捨てる"
    );
}

#[test]
fn form_text_is_kept_and_table_cells_are_separated() {
    let p =
        extract("<form><p>申込の流れ</p></form><table><tr><td>経費</td><td>5日</td></tr></table>");
    assert!(p.markdown.contains("申込の流れ"));
    assert!(p.markdown.contains("経費 5日"), "{:?}", p.markdown);
}

#[test]
fn invisible_format_characters_are_removed_and_one_long_line_still_yields_text() {
    let p = extract("<p>a\u{202E}b\u{200B}c</p>");
    assert_eq!(p.markdown, "a b c\n");
    let long = format!("<p>{}</p>", "あ".repeat(MAX_DOC_BYTES));
    let p = extract(&long);
    assert!(p.truncated && !p.markdown.trim().is_empty());
    assert!(p.markdown.len() <= MAX_DOC_BYTES);
}
