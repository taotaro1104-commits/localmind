use super::*;

const OK: &str = r#"{"summary":[{"text":"予算を確認した","assignee":"","due":"","cited":[0,2000]}],
"decisions":[],
"action_items":[{"text":"見積書を送る","assignee":"田中","due":"来週金曜","cited":[2000]}],
"open_issues":[{"text":"増員の時期","assignee":"","due":"","cited":[]}]}"#;

#[test]
fn a_well_formed_output_becomes_raw_minutes() {
    let m = parse_minutes(OK).unwrap();
    assert_eq!(m.summary[0].cited_start_ms, [0, 2000]);
    assert_eq!(m.summary[0].assignee, None, "空文字は「なし」");
    assert!(m.decisions.is_empty());
    assert_eq!(m.action_items[0].assignee.as_deref(), Some("田中"));
    assert_eq!(m.action_items[0].due.as_deref(), Some("来週金曜"));
    assert!(m.open_issues[0].cited_start_ms.is_empty(), "検証で落ちる");
}

#[test]
fn malformed_outputs_are_errors_not_panics() {
    for bad in [
        "",
        "not json",
        "{",
        r#"{"summary":[]}"#,
        r#"{"summary":[],"decisions":[],"action_items":[],"open_issues":{}}"#,
        r#"{"summary":[{"cited":[1]}],"decisions":[],"action_items":[],"open_issues":[]}"#,
        r#"{"summary":[{"text":"a"}],"decisions":[],"action_items":[],"open_issues":[]}"#,
        r#"{"summary":[{"text":1,"cited":[]}],"decisions":[],"action_items":[],"open_issues":[]}"#,
    ] {
        assert!(
            matches!(parse_minutes(bad), Err(IngestError::Extract(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn junk_citations_are_dropped_not_trusted() {
    let m = parse_minutes(
        r#"{"summary":[{"text":"a","assignee":"","due":"","cited":[-1,1.5,"x",null,7]}],
"decisions":[],"action_items":[],"open_issues":[]}"#,
    )
    .unwrap();
    assert_eq!(m.summary[0].cited_start_ms, [7]);
}

#[test]
fn an_oversized_output_is_refused_before_parsing() {
    let huge = " ".repeat(MAX_OUTPUT_BYTES + 1);
    assert!(parse_minutes(&huge).is_err());
}

#[test]
fn the_error_message_does_not_echo_the_model_output() {
    let err = parse_minutes("秘密の本文 not json")
        .unwrap_err()
        .to_string();
    assert!(!err.contains("秘密の本文"), "{err}");
}
