use super::*;

fn md(text: &str) -> Vec<Chunk> {
    split(text, Format::Markdown).unwrap()
}

#[test]
fn an_empty_document_has_no_chunks() {
    assert!(md("").is_empty());
    assert!(md("\n\n  \n").is_empty());
}

#[test]
fn headings_make_a_path_and_each_section_is_its_own_chunk() {
    let doc = "# 契約\n\n本文A\n\n## 解除\n\n本文B\n\n## 期間\n\n本文C\n\n# 付則\n\n本文D\n";
    let c = md(doc);
    let got: Vec<(Vec<&str>, &str)> = c
        .iter()
        .map(|c| {
            (
                c.heading_path.iter().map(String::as_str).collect(),
                c.text.as_str(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (vec!["契約"], "本文A"),
            (vec!["契約", "解除"], "本文B"),
            (vec!["契約", "期間"], "本文C"),
            (vec!["付則"], "本文D"),
        ]
    );
    assert_eq!(c.iter().map(|c| c.seq).collect::<Vec<_>>(), [0, 1, 2, 3]);
}

#[test]
fn line_numbers_point_at_the_original_lines() {
    let doc = "# 見出し\n\n一行目\n二行目\n\n次の段落\n";
    let c = md(doc);
    assert_eq!(c.len(), 1, "短い段落は束ねる");
    assert_eq!((c[0].start_line, c[0].end_line), (3, 6));
    let lines: Vec<&str> = doc.lines().collect();
    assert_eq!(lines[c[0].start_line - 1], "一行目");
    assert_eq!(lines[c[0].end_line - 1], "次の段落");
}

#[test]
fn short_paragraphs_are_merged_up_to_the_target_and_long_ones_stay_apart() {
    let a = "あ".repeat(300);
    let b = "い".repeat(300);
    let c = md(&format!("{a}\n\n{b}\n"));
    assert_eq!(c.len(), 2, "合わせて目標を超えるなら束ねない");
    let small = md("甲\n\n乙\n\n丙\n");
    assert_eq!(small.len(), 1);
    assert_eq!(small[0].text, "甲\n\n乙\n\n丙");
}

#[test]
fn a_long_paragraph_is_cut_at_sentence_ends() {
    let sentence = format!("{}。", "あ".repeat(99));
    let text = sentence.repeat(20); // 2000 文字
    let c = md(&text);
    assert!(c.len() >= 4, "{}", c.len());
    for chunk in &c {
        let n = chunk.text.chars().count();
        assert!(n <= MAX_CHARS, "{n}");
        assert!(chunk.text.ends_with('。'), "文の途中で切らない");
    }
    let joined: String = c.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(joined, text, "欠けも重なりも無い");
}

#[test]
fn text_without_sentence_ends_is_cut_by_length() {
    let text = "あ".repeat(2000);
    let c = split(&text, Format::Text).unwrap();
    assert!(c.iter().all(|c| c.text.chars().count() <= MAX_CHARS));
    assert_eq!(
        c.iter().map(|c| c.text.chars().count()).sum::<usize>(),
        2000
    );
}

#[test]
fn line_numbers_of_split_pieces_follow_the_newlines() {
    let line = format!("{}。", "あ".repeat(499));
    let text = format!("{line}\n{line}\n{line}");
    let c = split(&text, Format::Text).unwrap();
    assert_eq!(c.len(), 3);
    assert_eq!(
        c.iter().map(|c| c.start_line).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(c.iter().map(|c| c.end_line).collect::<Vec<_>>(), [1, 2, 3]);
}

#[test]
fn hash_lines_inside_a_code_fence_are_not_headings() {
    let doc = "# 手順\n\n```sh\n# これはコメント\n\necho hi\n```\n\n## 次\n\n本文\n";
    let c = md(doc);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].heading_path, ["手順"]);
    assert!(c[0].text.contains("# これはコメント"));
    assert!(c[0].text.contains("echo hi"), "コードの中の空行で割らない");
    assert_eq!(c[1].heading_path, ["手順", "次"]);
}

#[test]
fn only_real_headings_count() {
    // `#` の後に空白が無い・インデントが深い・題名が空の行は、見出しではない。
    let c = md("#タグ\n\n    # コード\n\n#\n\n####### 七つ\n");
    assert!(c.iter().all(|c| c.heading_path.is_empty()), "{c:?}");
    assert_eq!(c.len(), 1);
}

#[test]
fn closing_hashes_are_dropped_from_the_title() {
    let c = md("## 題名 ##\n\n本文\n");
    assert_eq!(c[0].heading_path, ["題名"]);
}

#[test]
fn plain_text_ignores_markdown_syntax() {
    let c = split("# これは見出しではない\n\n本文\n", Format::Text).unwrap();
    assert!(c[0].heading_path.is_empty());
    assert!(c[0].text.contains("# これは見出しではない"));
}

#[test]
fn crlf_is_handled_like_lf() {
    let a = md("# 題\r\n\r\n一\r\n二\r\n");
    let b = md("# 題\n\n一\n二\n");
    assert_eq!(a, b);
    assert_eq!(a[0].text, "一\n二");
}

#[test]
fn a_heading_without_a_body_makes_no_chunk() {
    assert!(md("# 空\n\n## これも空\n").is_empty());
}

#[test]
fn indexed_text_carries_the_heading_path() {
    let c = md("# 契約\n\n## 解除\n\n30日前に通知する。\n");
    assert_eq!(c[0].indexed_text(), "契約 > 解除\n30日前に通知する。");
    let t = split("本文だけ", Format::Text).unwrap();
    assert_eq!(t[0].indexed_text(), "本文だけ");
}

#[test]
fn invalid_input_is_an_error_not_a_panic() {
    assert!(matches!(
        split(&"a".repeat(MAX_DOC_BYTES + 1), Format::Text),
        Err(WorkspaceError::TooLarge { .. })
    ));
    assert!(matches!(
        split("a\0b", Format::Text),
        Err(WorkspaceError::InvalidText(_))
    ));
    let many = "# h\nx\n\n".repeat(MAX_CHUNKS + 1);
    assert!(matches!(
        split(&many, Format::Markdown),
        Err(WorkspaceError::TooManyChunks { .. })
    ));
}

#[test]
fn splitting_is_deterministic() {
    let doc = "# 題\n\n".to_string() + &"あいう。".repeat(500);
    assert_eq!(md(&doc), md(&doc));
}

#[test]
fn a_huge_heading_is_cut_so_it_cannot_be_multiplied_across_chunks() {
    let heading = "あ".repeat(5000);
    let body = "い".repeat(500);
    let doc = format!("# {heading}\n\n{body}\n\n{body}\n\n{body}\n");
    let c = md(&doc);
    assert!(c.len() >= 3);
    for chunk in &c {
        assert_eq!(chunk.heading_path[0].chars().count(), MAX_HEADING_CHARS);
        assert!(chunk.indexed_text().chars().count() <= MAX_HEADING_CHARS + 1 + MAX_CHARS);
    }
}

#[test]
fn closing_hashes_are_removed_only_after_a_space() {
    assert_eq!(md("# C#\n\n本文\n")[0].heading_path, ["C#"]);
    assert_eq!(md("## F# ##\n\n本文\n")[0].heading_path, ["F#"]);
    assert_eq!(md("# Issue #3#\n\n本文\n")[0].heading_path, ["Issue #3#"]);
    assert!(
        md("# #\n\n本文\n")[0].heading_path.is_empty(),
        "題名が空なら見出しではない"
    );
}

#[test]
fn a_leading_bom_does_not_hide_the_first_heading() {
    let c = md("\u{feff}# 題\n\n本文\n");
    assert_eq!(c[0].heading_path, ["題"]);
    assert_eq!(c[0].start_line, 3);
}

#[test]
fn code_fences_follow_the_commonmark_rules() {
    // 行内のコード（``` が同じ行で閉じる）は、フェンスの開始ではない。
    let c = md("# 題\n\n```x``` の例\n\n## 次\n\n本文\n");
    assert_eq!(c.last().unwrap().heading_path, ["題", "次"]);
    // 4 つで開いたら、3 つでは閉じない。
    let c = md("# 題\n\n````\n```\n## 中\n````\n\n## 次\n\n本文\n");
    assert_eq!(c.last().unwrap().heading_path, ["題", "次"]);
    assert!(
        c[0].text.contains("## 中"),
        "フェンスの中の見出し風の行は本文"
    );
    // 情報文字列つきの ``` は、閉じる行ではない。
    let c = md("# 題\n\n```\n```rust\n## 中\n```\n\n## 次\n\n本文\n");
    assert!(c[0].text.contains("## 中"));
    assert_eq!(c.last().unwrap().heading_path, ["題", "次"]);
}

#[test]
fn an_unclosed_fence_runs_to_the_end_of_the_document() {
    let c = md("# 題\n\n```\nコード\n\n## 見出しに見えるが中身\n");
    assert_eq!(c.len(), 1);
    assert!(c[0].text.contains("## 見出しに見えるが中身"));
}

#[test]
fn splitting_a_long_paragraph_keeps_indentation_and_stays_fast() {
    let line = format!("    {}。", "あ".repeat(60));
    let text = std::iter::repeat_n(line.as_str(), 1000)
        .collect::<Vec<_>>()
        .join("\n");
    let started = std::time::Instant::now();
    let c = split(&text, Format::Text).unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(c.iter().all(|c| c.text.chars().count() <= MAX_CHARS));
    let lines: Vec<&str> = text.lines().collect();
    for chunk in &c {
        assert_eq!(
            lines[chunk.start_line - 1].trim(),
            chunk.text.lines().next().unwrap().trim()
        );
        assert_eq!(
            chunk.end_line - chunk.start_line,
            chunk.text.matches('\n').count()
        );
    }
    assert!(
        c[0].text.starts_with("    "),
        "先頭のインデントを落とさない"
    );
}

#[test]
fn fences_are_matched_by_marker_and_length() {
    // 短いフェンスでは閉じない。
    let c = md("# 題\n\n`````\n```\n## 中\n`````\n\n## 次\n\n本文\n");
    assert!(c[0].text.contains("## 中"));
    assert_eq!(c.last().unwrap().heading_path, ["題", "次"]);
    // ~~~ は ``` では閉じない。
    let c = md("# 題\n\n~~~\n```\n## 中\n~~~\n\n## 次\n\n本文\n");
    assert!(c[0].text.contains("## 中"));
    assert_eq!(c.last().unwrap().heading_path, ["題", "次"]);
    // バッククォートの情報文字列に ` があれば、開始ではない（見出しは生きる）。
    let c = md("# 題\n\n```a`b\n\n## 次\n\n本文\n");
    assert_eq!(c.last().unwrap().heading_path, ["題", "次"]);
}

#[test]
fn split_pieces_keep_exact_lines_at_japanese_sentence_ends() {
    let a = format!("{}。", "あ".repeat(450));
    let b = format!("{}。", "い".repeat(450));
    let text = format!("{a}\n{b}\n{a}");
    let c = split(&text, Format::Text).unwrap();
    assert_eq!(c.len(), 3);
    for (i, chunk) in c.iter().enumerate() {
        assert_eq!((chunk.start_line, chunk.end_line), (i + 1, i + 1), "{i}");
    }
}
