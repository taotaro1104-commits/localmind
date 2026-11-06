use super::*;
use std::path::PathBuf;
use std::sync::Arc;

/// テストごとの専用ディレクトリ。Drop で消す。
struct Dirs(PathBuf);

impl Dirs {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lm-ffi-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn index(&self) -> String {
        self.0.join("index.db").to_string_lossy().into_owned()
    }
    fn docs(&self) -> String {
        self.0.join("docs").to_string_lossy().into_owned()
    }
    fn open(&self) -> Arc<WorkspaceEngine> {
        WorkspaceEngine::open_lexical(self.index(), self.docs()).unwrap()
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const RULES: &str = "# 就業規則\n\n## 休暇\n\n年次有給休暇は入社6か月後に10日付与される。\n\n## 経費\n\n出張の経費精算は帰着後5営業日以内に申請する。\n";

#[test]
fn a_document_is_found_with_its_heading_and_line_numbers() {
    let d = Dirs::new("cite");
    let ws = d.open();
    assert_eq!(
        ws.add_document(
            "rules".into(),
            "就業規則".into(),
            DocFormat::Markdown,
            RULES.into(),
            None,
        )
        .unwrap(),
        2
    );
    let r = ws
        .search("経費精算は何日以内に申請しますか".into(), 3)
        .unwrap();
    assert!(r.found, "{r:?}");
    let c = &r.citations[0];
    assert_eq!((c.doc_id.as_str(), c.title.as_str()), ("rules", "就業規則"));
    assert_eq!(c.heading_path, ["就業規則", "経費"]);
    assert_eq!((c.start_line, c.end_line), (9, 9));
    assert!(c.excerpt.contains("5営業日以内"));
}

#[test]
fn a_question_with_no_answer_is_not_found() {
    let d = Dirs::new("nf");
    let ws = d.open();
    ws.add_document(
        "rules".into(),
        "就業規則".into(),
        DocFormat::Markdown,
        RULES.into(),
        None,
    )
    .unwrap();
    let r = ws.search("宇宙旅行の費用はいくらですか".into(), 3).unwrap();
    assert!(!r.found && r.citations.is_empty());
}

#[test]
fn documents_survive_reopening_and_can_be_listed_and_removed() {
    let d = Dirs::new("reopen");
    {
        let ws = d.open();
        ws.add_document(
            "rules".into(),
            "就業規則".into(),
            DocFormat::Markdown,
            RULES.into(),
            None,
        )
        .unwrap();
    }
    let ws = d.open();
    let docs = ws.documents().unwrap();
    assert_eq!(
        docs,
        [DocumentInfo {
            id: "rules".into(),
            title: "就業規則".into(),
            format: DocFormat::Markdown,
            group: String::new(),
            chunks: 2
        }]
    );
    assert!(
        ws.search("経費精算は何日以内".into(), 3).unwrap().found,
        "索引も残る"
    );
    assert!(ws.remove_document("rules".into()).unwrap());
    assert!(ws.documents().unwrap().is_empty());
    assert!(!ws.remove_document("rules".into()).unwrap());
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let d = Dirs::new("bad");
    let ws = d.open();
    let add = |id: &str, title: &str, text: &str| {
        ws.add_document(id.into(), title.into(), DocFormat::Text, text.into(), None)
    };
    assert!(matches!(
        add("../x", "題", "本文"),
        Err(FfiError::Workspace { .. })
    ));
    assert!(matches!(
        add("a", "", "本文"),
        Err(FfiError::Workspace { .. })
    ));
    assert!(matches!(
        add("a", "題", "a\0b"),
        Err(FfiError::Workspace { .. })
    ));
    let huge = "あ".repeat(400_000);
    assert!(matches!(
        add("a", "題", &huge),
        Err(FfiError::Workspace { .. })
    ));
    assert!(ws.documents().unwrap().is_empty(), "拒否した文書は残らない");
}

#[test]
fn relative_paths_are_refused_when_opening() {
    assert!(WorkspaceEngine::open_lexical("index.db".into(), "/tmp/docs".into()).is_err());
    assert!(WorkspaceEngine::open_lexical("/tmp/index.db".into(), "docs".into()).is_err());
    assert!(WorkspaceEngine::open_lexical("/tmp/../index.db".into(), "/tmp/docs".into()).is_err());
}

#[test]
fn reindex_restores_search_after_the_index_is_lost() {
    let d = Dirs::new("reindex");
    {
        let ws = d.open();
        ws.add_document(
            "rules".into(),
            "就業規則".into(),
            DocFormat::Markdown,
            RULES.into(),
            None,
        )
        .unwrap();
    }
    std::fs::remove_file(d.0.join("index.db")).unwrap();
    let ws = d.open();
    assert!(!ws.search("経費精算は何日以内".into(), 3).unwrap().found);
    assert!(ws.reindex().unwrap().is_empty());
    assert!(ws.search("経費精算は何日以内".into(), 3).unwrap().found);
}

#[test]
fn groups_are_set_listed_and_kept_when_a_document_is_replaced() {
    let d = Dirs::new("grp");
    let ws = d.open();
    ws.add_document(
        "rules".into(),
        "就業規則".into(),
        DocFormat::Markdown,
        RULES.into(),
        Some("人事".into()),
    )
    .unwrap();
    ws.add_document(
        "memo".into(),
        "メモ".into(),
        DocFormat::Text,
        "本文".into(),
        None,
    )
    .unwrap();
    let groups = |ws: &WorkspaceEngine| {
        ws.documents()
            .unwrap()
            .into_iter()
            .map(|d| (d.id, d.group))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        groups(&ws),
        vec![("memo".into(), "".into()), ("rules".into(), "人事".into())]
    );
    assert!(ws.set_document_group("memo".into(), "個人".into()).unwrap());
    assert!(!ws.set_document_group("zzz".into(), "個人".into()).unwrap());
    assert!(ws.set_document_group("memo".into(), "a\nb".into()).is_err());
    ws.add_document(
        "rules".into(),
        "就業規則".into(),
        DocFormat::Markdown,
        RULES.into(),
        None,
    )
    .unwrap();
    assert_eq!(groups(&ws)[1].1, "人事", "置き換えても、グループは残る");
}

#[test]
fn html_pages_become_titled_markdown() {
    let p = html_to_markdown(
        "<html><head><title>規程</title></head><body><h1>規程</h1><p>本文&amp;注意</p><script>x()</script></body></html>"
            .into(),
    );
    assert_eq!(p.title, "規程");
    assert_eq!(p.markdown, "# 規程\n\n本文&注意\n");
    assert!(!p.truncated);
}
