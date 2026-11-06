use super::*;
use lm_search::{Embedder, HashingEmbedder, HybridConfig, NgramTokenizer};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU32, Ordering};

fn temp_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "lm-wsp-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// 止められる埋め込み（索引の更新の失敗を起こすため）。
struct Flaky {
    inner: HashingEmbedder,
    fail: Arc<AtomicBool>,
}

impl Embedder for Flaky {
    fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(SearchError::Embedding("止めている".into()));
        }
        self.inner.embed(text)
    }
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn id(&self) -> String {
        "flaky".into()
    }
}

fn workspace(dir: &PathBuf) -> Workspace {
    workspace_with(dir, Box::new(HashingEmbedder::new(512)))
}

fn workspace_with(dir: &PathBuf, embedder: Box<dyn Embedder>) -> Workspace {
    let store = Arc::new(SqliteStore::in_memory(Box::new(NgramTokenizer), embedder).unwrap());
    let searcher = HybridSearcher::new(store.clone(), store.clone(), HybridConfig::default());
    Workspace::new(store, searcher, DocStore::new(dir))
}

const REGULATIONS: &str = "# 就業規則\n\n## 休暇\n\n年次有給休暇は入社6か月後に10日付与される。\n\n## 経費\n\n出張の経費精算は帰着後5営業日以内に申請する。\n";

#[test]
fn an_added_document_is_found_with_heading_and_lines() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    assert_eq!(
        ws.add("rules", "就業規則", Format::Markdown, REGULATIONS)
            .unwrap(),
        2
    );
    let r = ws.search("経費精算は何日以内に申請しますか", 3).unwrap();
    assert!(r.found, "{r:?}");
    let top = &r.citations[0];
    assert_eq!(top.doc_id, "rules");
    assert_eq!(top.title, "就業規則");
    assert_eq!(top.heading_path, ["就業規則", "経費"]);
    assert_eq!((top.start_line, top.end_line), (9, 9));
    assert!(top.excerpt.contains("5営業日以内"));
    let line = REGULATIONS.lines().nth(top.start_line - 1).unwrap();
    assert!(line.contains("経費精算"), "出典の行に実際に書いてある");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_question_the_documents_cannot_answer_is_not_found() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("rules", "就業規則", Format::Markdown, REGULATIONS)
        .unwrap();
    let r = ws.search("宇宙旅行の費用はいくらですか", 3).unwrap();
    assert!(!r.found);
    assert!(r.citations.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn replacing_a_document_drops_the_old_chunks() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add(
        "d",
        "題",
        Format::Markdown,
        "# A\n\n旧文書だけにある語りんご\n\n# B\n\n二つ目\n",
    )
    .unwrap();
    assert_eq!(
        ws.add("d", "題", Format::Markdown, "# A\n\n新しい内容みかん\n")
            .unwrap(),
        1
    );
    assert_eq!(ws.store.len().unwrap(), 1, "余りのチャンクが消えている");
    assert!(!ws.search("りんご", 3).unwrap().found);
    assert!(ws.search("みかん", 3).unwrap().found);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn removing_a_document_removes_it_from_search_and_disk() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("rules", "就業規則", Format::Markdown, REGULATIONS)
        .unwrap();
    assert!(ws.remove("rules").unwrap());
    assert!(!ws.remove("rules").unwrap());
    assert_eq!(ws.store.len().unwrap(), 0);
    assert!(!ws.search("経費精算", 3).unwrap().found);
    assert!(ws.list().unwrap().is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bad_input_is_refused_before_anything_is_written() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    assert!(matches!(
        ws.add("a/b", "題", Format::Text, "x"),
        Err(WorkspaceError::InvalidId)
    ));
    assert!(matches!(
        ws.add("a", "", Format::Text, "x"),
        Err(WorkspaceError::InvalidTitle(_))
    ));
    assert!(matches!(
        ws.add("a", "題", Format::Text, "x\0y"),
        Err(WorkspaceError::InvalidText(_))
    ));
    assert_eq!(ws.store.len().unwrap(), 0);
    assert!(!dir.exists());
}

#[test]
fn list_shows_titles_and_chunk_counts_in_id_order() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("b", "二番", Format::Text, "本文").unwrap();
    ws.add("a", "一番", Format::Markdown, REGULATIONS).unwrap();
    let list = ws.list().unwrap();
    assert_eq!(
        list.iter()
            .map(|d| (d.id.as_str(), d.title.as_str(), d.chunks))
            .collect::<Vec<_>>(),
        [("a", "一番", 2), ("b", "二番", 1)]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reindex_rebuilds_the_index_from_the_stored_documents() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("rules", "就業規則", Format::Markdown, REGULATIONS)
        .unwrap();
    // 別の（空の）索引で開き直した状況: 原本だけが残っている。
    let ws2 = workspace(&dir);
    assert!(!ws2.search("経費精算", 3).unwrap().found);
    assert!(ws2.reindex().unwrap().is_empty());
    assert!(ws2.search("経費精算は何日以内", 3).unwrap().found);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reindex_skips_unreadable_documents_and_reports_them() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("ok", "題", Format::Text, "読める文書です").unwrap();
    fs::write(dir.join("broken.doc"), "壊れている").unwrap();
    assert_eq!(ws.reindex().unwrap(), ["broken"]);
    assert_eq!(ws.list().unwrap().len(), 1, "壊れた文書は一覧に出さない");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_document_deleted_during_search_is_skipped_not_an_error() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("rules", "就業規則", Format::Markdown, REGULATIONS)
        .unwrap();
    fs::remove_file(dir.join("rules.doc")).unwrap(); // 索引にだけ残った状態
    let r = ws.search("経費精算", 3).unwrap();
    assert!(!r.found && r.citations.is_empty(), "{r:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn chunk_ids_round_trip_and_reject_foreign_ids() {
    assert_eq!(parse_chunk_id(&chunk_id("doc-1", 42)), Some(("doc-1", 42)));
    assert_eq!(parse_chunk_id("no-separator"), None);
    assert_eq!(parse_chunk_id("a~x"), None);
}

#[test]
fn excerpts_are_one_line_and_capped() {
    assert_eq!(excerpt("一行目\n\n  二行目"), "一行目 二行目");
    let long = "あ".repeat(500);
    let e = excerpt(&long);
    assert_eq!(e.chars().count(), EXCERPT_CHARS + 1);
    assert!(e.ends_with('…'));
}

#[test]
fn shrinking_a_document_removes_its_leftover_index_rows() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    let long: String = (0..6)
        .map(|i| format!("# 節{i}\n\n本文その{i}\n\n"))
        .collect();
    assert_eq!(ws.add("d", "題", Format::Markdown, &long).unwrap(), 6);
    assert_eq!(
        ws.add("d", "題", Format::Markdown, "# 節0\n\n本文その0\n")
            .unwrap(),
        1
    );
    assert_eq!(ws.store.ids_with_prefix("d~").unwrap(), ["d~00000"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn remove_clears_the_index_even_when_the_original_is_damaged() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("rules", "就業規則", Format::Markdown, REGULATIONS)
        .unwrap();
    fs::write(dir.join("rules.doc"), "壊れている").unwrap();
    assert!(ws.remove("rules").unwrap());
    assert_eq!(ws.store.len().unwrap(), 0, "孤児の索引を残さない");
    assert!(!dir.join("rules.doc").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reindex_drops_rows_whose_original_is_gone() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("a", "一", Format::Text, "残る文書です").unwrap();
    ws.add("b", "二", Format::Text, "消える文書です").unwrap();
    fs::remove_file(dir.join("b.doc")).unwrap();
    assert!(ws.reindex().unwrap().is_empty());
    assert_eq!(ws.store.ids_with_prefix("").unwrap(), ["a~00000"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_index_update_is_reported_and_reindex_repairs_it() {
    let dir = temp_dir();
    let fail = Arc::new(AtomicBool::new(false));
    let ws = workspace_with(
        &dir,
        Box::new(Flaky {
            inner: HashingEmbedder::new(512),
            fail: fail.clone(),
        }),
    );
    ws.add("d", "題", Format::Text, "古い内容りんご").unwrap();
    fail.store(true, Ordering::Relaxed);
    let err = ws
        .add("d", "題", Format::Text, "新しい内容みかん")
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::IndexOutOfDate(_)), "{err:?}");
    assert_eq!(
        ws.docs.load("d").unwrap().text,
        "新しい内容みかん",
        "原本は新しい版"
    );
    fail.store(false, Ordering::Relaxed);
    ws.reindex().unwrap();
    assert!(ws.search("みかん", 3).unwrap().found);
    assert!(
        !ws.search("りんご", 3).unwrap().found,
        "古い内容は索引に残らない"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_number_of_documents_is_capped_but_replacing_one_is_still_allowed() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    for i in 0..MAX_DOCS {
        ws.add(&format!("d{i}"), "題", Format::Text, "本文")
            .unwrap();
    }
    let err = ws.add("extra", "題", Format::Text, "本文").unwrap_err();
    assert!(matches!(err, WorkspaceError::Full { .. }), "{err:?}");
    assert!(ws.add("d0", "題", Format::Text, "置き換え").is_ok());
    assert!(ws.remove("d1").unwrap());
    assert!(ws.add("extra", "題", Format::Text, "本文").is_ok());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_chunker_version_rebuilds_the_index_once() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    assert!(
        !ws.ensure_current().unwrap(),
        "空のワークスペースは作り直さない"
    );
    ws.add("rules", "就業規則", Format::Markdown, REGULATIONS)
        .unwrap();
    assert!(!ws.ensure_current().unwrap());
    ws.docs.write_version(0).unwrap(); // 古い版で作った索引、という状況
    ws.store.remove("rules~00001").unwrap();
    assert!(ws.ensure_current().unwrap());
    assert_eq!(ws.store.len().unwrap(), 2, "索引が直っている");
    assert!(!ws.ensure_current().unwrap(), "2 回目は何もしない");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ids_that_collide_on_case_insensitive_systems_are_refused() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    assert!(matches!(
        ws.add("Doc", "題", Format::Text, "x"),
        Err(WorkspaceError::InvalidId)
    ));
    assert!(matches!(
        ws.add("con", "題", Format::Text, "x"),
        Err(WorkspaceError::InvalidId)
    ));
    assert!(!dir.exists());
}

#[test]
fn k_is_capped_and_repeated_hits_in_one_document_are_all_cited() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    let doc: String = (0..5)
        .map(|i| format!("# 節{i}\n\n経費精算の規定その{i}。申請は5営業日以内。\n\n"))
        .collect();
    ws.add("rules", "規程", Format::Markdown, &doc).unwrap();
    let r = ws.search("経費精算の申請", 100_000).unwrap();
    assert!(r.found);
    assert!(
        r.citations.len() > 1,
        "同じ文書の複数の箇所を引用できる: {}",
        r.citations.len()
    );
    assert!(r.citations.iter().all(|c| c.doc_id == "rules"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ids_with_underscores_and_hyphens_do_not_touch_each_other() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    for id in ["a_b", "axb", "a-b", "a"] {
        ws.add(id, "題", Format::Text, &format!("{id} の本文"))
            .unwrap();
    }
    assert!(ws.remove("a_b").unwrap());
    assert_eq!(
        ws.store.ids_with_prefix("").unwrap(),
        ["a-b~00000", "axb~00000", "a~00000"]
    );
    ws.add("a", "題", Format::Text, "置き換え").unwrap();
    assert_eq!(ws.store.len().unwrap(), 3);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn adding_before_ensure_current_does_not_hide_an_old_index() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("rules", "就業規則", Format::Markdown, REGULATIONS)
        .unwrap();
    fs::remove_file(dir.join("chunker.version")).unwrap(); // 版の記録が消えた状況
    ws.add("other", "題", Format::Text, "別の文書です").unwrap();
    assert_eq!(
        ws.docs.read_version(),
        None,
        "文書があるワークスペースでは、add は版を書かない"
    );
    assert!(ws.ensure_current().unwrap(), "作り直しが行われる");
    assert_eq!(ws.docs.read_version(), Some(CHUNKER_VERSION));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reindex_does_not_record_the_version_while_a_document_is_unreadable() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add("ok", "題", Format::Text, "読める文書です").unwrap();
    ws.docs.write_version(0).unwrap();
    fs::write(dir.join("broken.doc"), "壊れている").unwrap();
    assert_eq!(ws.reindex().unwrap(), ["broken"]);
    assert_eq!(
        ws.docs.read_version(),
        Some(0),
        "次に開いたときにもう一度作り直す"
    );
    fs::remove_file(dir.join("broken.doc")).unwrap();
    assert!(ws.reindex().unwrap().is_empty());
    assert_eq!(ws.docs.read_version(), Some(CHUNKER_VERSION));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn leftover_index_rows_are_cleaned_even_when_no_documents_remain() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.store.upsert("ghost~00000", "孤児の行").unwrap(); // 原本の無い索引の行
    ws.store.upsert("other-use", "別の用途の行").unwrap(); // この層のものではない行
    assert!(ws.ensure_current().unwrap());
    assert_eq!(
        ws.store.ids_with_prefix("").unwrap(),
        ["other-use"],
        "この層の孤児だけを消す"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn weak_hits_are_cut_by_the_distance_from_the_top_and_the_top_hit_always_stays() {
    let build = |margin: f32| {
        let dir = temp_dir();
        let ws = workspace(&dir).with_relevance_margin(margin);
        for (id, text) in [
            ("a", "出張の経費精算は帰着後5営業日以内に申請する。"),
            ("b", "出張の旅費の精算方法と申請の期限について。"),
            ("c", "経費の精算に使う領収書は原本を提出する。"),
            ("d", "年次有給休暇は入社6か月後に10日付与される。"),
        ] {
            ws.add(id, "題", Format::Text, text).unwrap();
        }
        (ws, dir)
    };
    let (loose, d1) = build(1.0);
    let (strict, d2) = build(0.0);
    let all = loose.search("出張の経費精算の申請", 4).unwrap();
    let cut = strict.search("出張の経費精算の申請", 4).unwrap();
    assert!(all.citations.len() >= 2, "{all:?}");
    assert!(
        cut.citations.len() < all.citations.len(),
        "{} / {}",
        cut.citations.len(),
        all.citations.len()
    );
    assert_eq!(
        cut.citations[0].doc_id, all.citations[0].doc_id,
        "1 位は変わらない"
    );
    let _ = (fs::remove_dir_all(&d1), fs::remove_dir_all(&d2));
}

#[test]
fn the_margin_setter_rejects_nonsense() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    assert_eq!(ws.relevance_margin, f32::INFINITY, "既定は足切りなし");
    assert_eq!(
        workspace(&dir).with_relevance_margin(-1.0).relevance_margin,
        0.0
    );
    assert_eq!(
        workspace(&dir)
            .with_relevance_margin(f32::NAN)
            .relevance_margin,
        f32::INFINITY
    );
    assert_eq!(
        workspace(&dir).with_relevance_margin(0.1).relevance_margin,
        0.1
    );
}

/// 狙った cosine になる固定の埋め込み（足切りを決定的に試すため）。
struct Fixed;

impl Embedder for Fixed {
    fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        Ok(if text.contains("経費") {
            vec![1.0, 0.0, 0.0, 0.0] // 基準
        } else if text.contains("旅費") {
            vec![0.995, 0.099_875, 0.0, 0.0] // 基準との cosine 0.995
        } else {
            vec![0.9, 0.435_89, 0.0, 0.0] // 基準との cosine 0.9
        })
    }
    fn dim(&self) -> usize {
        4
    }
    fn id(&self) -> String {
        "fixed".into()
    }
}

fn fixed_workspace(margin: f32) -> (Workspace, PathBuf) {
    let dir = temp_dir();
    let ws = workspace_with(&dir, Box::new(Fixed)).with_relevance_margin(margin);
    for (id, text) in [
        ("d1", "2024年版 経費精算の規定です"),
        ("d2", "2024年版 旅費の精算方法です"),
        ("d3", "2024年版 休暇の規定です"),
    ] {
        ws.add(id, "題", Format::Text, text).unwrap();
    }
    (ws, dir)
}

fn doc_ids(r: &Report) -> Vec<&str> {
    let mut ids: Vec<&str> = r.citations.iter().map(|c| c.doc_id.as_str()).collect();
    ids.sort_unstable();
    ids
}

#[test]
fn only_hits_within_the_margin_of_the_top_cosine_are_kept() {
    let (ws, dir) = fixed_workspace(0.03);
    let r = ws.search("経費の規定", 3).unwrap();
    assert_eq!(
        doc_ids(&r),
        ["d1", "d2"],
        "0.995 は残り、0.9 は落ちる: {r:?}"
    );
    assert_eq!(r.citations[0].doc_id, "d1", "1 位は変わらない");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn without_a_margin_nothing_is_cut() {
    let (ws, dir) = fixed_workspace(f32::INFINITY);
    assert_eq!(
        doc_ids(&ws.search("経費の規定", 3).unwrap()),
        ["d1", "d2", "d3"]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_query_with_digits_is_never_cut_because_lexical_matches_matter_there() {
    let (ws, dir) = fixed_workspace(0.0);
    let r = ws.search("経費の規定 2024", 3).unwrap();
    assert_eq!(doc_ids(&r), ["d1", "d2", "d3"], "{r:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_tie_with_the_top_is_kept_and_a_single_hit_is_returned_as_is() {
    let (ws, dir) = fixed_workspace(0.0);
    ws.add("d4", "題", Format::Text, "経費の別の規定です")
        .unwrap(); // 1 位と同じ cosine（1.0）
    let r = ws.search("経費の規定", 4).unwrap();
    assert!(
        doc_ids(&r).contains(&"d1") && doc_ids(&r).contains(&"d4"),
        "{r:?}"
    );
    assert!(!doc_ids(&r).contains(&"d3"));
    let one = ws.search("休暇", 1).unwrap();
    assert_eq!(one.citations.len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn documents_carry_a_group_that_can_be_changed_without_touching_the_index() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add_with(
        "a",
        "就業規則",
        Format::Markdown,
        REGULATIONS,
        Some(" 人事 "),
    )
    .unwrap();
    ws.add("b", "メモ", Format::Text, "ただのメモです").unwrap();
    let list = ws.list().unwrap();
    assert_eq!(list[0].group, "人事", "前後の空白は除く");
    assert_eq!(list[1].group, "", "指定が無ければ未分類");

    assert!(ws.set_group("b", "個人").unwrap());
    assert!(!ws.set_group("none", "個人").unwrap(), "無い文書は false");
    assert_eq!(ws.list().unwrap()[1].group, "個人");
    // グループを変えても、検索は変わらない
    let r = ws.search("出張の経費精算はいつまで", 3).unwrap();
    assert!(r.found && r.citations[0].doc_id == "a");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn replacing_a_document_keeps_its_group_unless_a_new_one_is_given() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add_with("a", "題", Format::Text, "古い", Some("人事"))
        .unwrap();
    ws.add("a", "題", Format::Text, "新しい").unwrap();
    assert_eq!(ws.list().unwrap()[0].group, "人事");
    ws.add_with("a", "題", Format::Text, "さらに新しい", Some(""))
        .unwrap();
    assert_eq!(ws.list().unwrap()[0].group, "", "空を渡せば未分類に戻せる");
    ws.add_with("a", "題", Format::Text, "もう一度", Some("経理"))
        .unwrap();
    assert_eq!(ws.list().unwrap()[0].group, "経理");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bad_group_names_are_refused_before_anything_is_written() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    for bad in ["a\nb", "あ".repeat(41).as_str()] {
        assert!(matches!(
            ws.add_with("a", "題", Format::Text, "x", Some(bad)),
            Err(WorkspaceError::InvalidTitle(_))
        ));
        assert!(ws.set_group("a", bad).is_err());
    }
    assert!(ws.list().unwrap().is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn searching_within_a_group_keeps_only_that_groups_hits() {
    let dir = temp_dir();
    let ws = workspace(&dir);
    ws.add_with(
        "hr",
        "就業規則",
        Format::Markdown,
        REGULATIONS,
        Some("人事"),
    )
    .unwrap();
    ws.add_with(
        "acc",
        "経理規程",
        Format::Markdown,
        "# 経理規程\n\n## 経費\n\n出張の経費精算は帰着後5営業日以内に申請する。\n",
        Some("経理"),
    )
    .unwrap();
    ws.add_with(
        "none",
        "メモ",
        Format::Text,
        "出張の経費精算の覚え書き",
        None,
    )
    .unwrap();
    let q = "出張の経費精算はいつまでに申請しますか";

    let all: Vec<String> = ws
        .search(q, 10)
        .unwrap()
        .citations
        .into_iter()
        .map(|c| c.doc_id)
        .collect();
    assert!(all.contains(&"hr".to_string()) && all.contains(&"acc".to_string()));

    for (group, expected) in [("人事", "hr"), ("経理", "acc"), ("", "none")] {
        let r = ws.search_in(q, 10, Some(group)).unwrap();
        assert!(r.found, "{group}");
        assert!(
            r.citations.iter().all(|c| c.doc_id == expected),
            "{group}: {:?}",
            r.citations.iter().map(|c| &c.doc_id).collect::<Vec<_>>()
        );
    }
    // 存在しないグループは、何も見つからない（ほかのグループの結果は出ない）
    let r = ws.search_in(q, 10, Some("営業")).unwrap();
    assert!(!r.found && r.citations.is_empty());
    // k は絞ったあとの件数に効く
    assert_eq!(ws.search_in(q, 1, Some("人事")).unwrap().citations.len(), 1);
    assert!(ws.search_in(q, 3, Some("a\nb")).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_scoped_search_never_turns_a_global_miss_into_an_answer() {
    let dir = temp_dir();
    // 足切りを厳しくして、1 位と同じ強さの命中だけを残す
    let ws = workspace(&dir).with_relevance_margin(0.0);
    ws.add_with(
        "hr",
        "就業規則",
        Format::Markdown,
        REGULATIONS,
        Some("人事"),
    )
    .unwrap();
    ws.add_with(
        "acc",
        "経理メモ",
        Format::Text,
        "請求書の発行は月末にまとめて行います",
        Some("経理"),
    )
    .unwrap();
    let q = "年次有給休暇はいつ付与されますか";
    assert!(ws.search_in(q, 5, Some("人事")).unwrap().found);
    // 絞った結果には、指定したグループの資料だけが入る（語彙だけで拾われた命中は、足切りの対象外）
    let r = ws.search_in(q, 5, Some("経理")).unwrap();
    assert!(
        r.citations.iter().all(|c| c.doc_id == "acc"),
        "{:?}",
        r.citations
    );
    // 全体で見つからない質問は、どのグループに絞っても見つからない
    let none = "宇宙旅行の費用はいくらですか";
    assert!(!ws.search(none, 5).unwrap().found);
    for g in ["人事", "経理", ""] {
        assert!(!ws.search_in(none, 5, Some(g)).unwrap().found, "{g}");
    }
    let _ = fs::remove_dir_all(&dir);
}
