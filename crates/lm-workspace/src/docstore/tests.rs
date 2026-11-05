use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lm-ws-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn doc(id: &str, text: &str) -> StoredDoc {
    StoredDoc {
        id: id.into(),
        title: "題名".into(),
        format: Format::Markdown,
        text: text.into(),
    }
}

#[test]
fn a_saved_document_comes_back_unchanged() {
    let dir = temp_dir("rt");
    let store = DocStore::new(&dir);
    let d = doc("contract-1", "# 見出し\n\n本文\n\n\n末尾の空行も保つ\n");
    store.save(&d).unwrap();
    assert_eq!(store.load("contract-1").unwrap(), d);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn saving_again_replaces_the_document_and_leaves_no_temp_files() {
    let dir = temp_dir("replace");
    let store = DocStore::new(&dir);
    store.save(&doc("a", "古い")).unwrap();
    store.save(&doc("a", "新しい")).unwrap();
    assert_eq!(store.load("a").unwrap().text, "新しい");
    let names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["a.doc"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ids_are_limited_so_they_cannot_escape_the_directory() {
    for bad in [
        "",
        "..",
        "a/b",
        "a\\b",
        "a.b",
        "a~1",
        "日本語",
        &"x".repeat(65),
    ] {
        assert!(
            matches!(check_id(bad), Err(WorkspaceError::InvalidId)),
            "{bad:?}"
        );
        let store = DocStore::new(temp_dir("id"));
        assert!(store.save(&doc(bad, "x")).is_err(), "{bad:?}");
        assert!(store.load(bad).is_err(), "{bad:?}");
        assert!(store.remove(bad).is_err(), "{bad:?}");
    }
    assert!(check_id("ab-9_z").is_ok());
}

#[test]
fn titles_must_be_one_short_line() {
    let dir = temp_dir("title");
    let store = DocStore::new(&dir);
    for bad in [
        "",
        "  ",
        "改行\nあり",
        "tab\tあり",
        &"あ".repeat(MAX_TITLE_CHARS + 1),
    ] {
        let mut d = doc("t", "x");
        d.title = bad.to_string();
        assert!(
            matches!(store.save(&d), Err(WorkspaceError::InvalidTitle(_))),
            "{bad:?}"
        );
    }
    assert!(!dir.join("t.doc").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_document_is_not_found_and_remove_reports_it() {
    let dir = temp_dir("missing");
    let store = DocStore::new(&dir);
    assert!(matches!(store.load("nope"), Err(WorkspaceError::NotFound)));
    assert!(!store.remove("nope").unwrap());
    store.save(&doc("x", "y")).unwrap();
    assert!(store.remove("x").unwrap());
    assert!(matches!(store.load("x"), Err(WorkspaceError::NotFound)));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_damaged_file_is_an_error_not_a_panic() {
    let dir = temp_dir("bad");
    fs::create_dir_all(&dir).unwrap();
    let store = DocStore::new(&dir);
    for (i, content) in [
        "",
        "LMDOC0\nformat=text\ntitle=t\n\nx",
        "LMDOC1\nformat=pdf\ntitle=t\n\nx",
        "LMDOC1\nformat=text\nname=t\n\nx",
        "LMDOC1\nformat=text\ntitle=t\nx",
        "LMDOC1\nformat=text\ntitle=\n\nx",
    ]
    .iter()
    .enumerate()
    {
        let id = format!("bad{i}");
        fs::write(dir.join(format!("{id}.doc")), content).unwrap();
        assert!(
            matches!(store.load(&id), Err(WorkspaceError::Corrupt)),
            "{content:?}"
        );
    }
    fs::write(
        dir.join("big.doc"),
        "x".repeat(MAX_DOC_BYTES + MAX_HEADER_BYTES + 2),
    )
    .unwrap();
    assert!(matches!(store.load("big"), Err(WorkspaceError::Corrupt)));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn oversized_text_is_refused_before_anything_is_written() {
    let dir = temp_dir("big");
    let store = DocStore::new(&dir);
    let err = store
        .save(&doc("a", &"x".repeat(MAX_DOC_BYTES + 1)))
        .unwrap_err();
    assert!(matches!(err, WorkspaceError::TooLarge { .. }));
    assert!(!dir.exists());
}

#[test]
fn ids_lists_only_valid_documents_in_order() {
    let dir = temp_dir("ids");
    let store = DocStore::new(&dir);
    assert!(store.ids().unwrap().is_empty(), "保存先が無くても空");
    store.save(&doc("b", "1")).unwrap();
    store.save(&doc("a", "2")).unwrap();
    fs::write(dir.join("notes.txt"), "x").unwrap();
    fs::write(dir.join("bad id.doc"), "x").unwrap();
    fs::create_dir_all(dir.join("sub.doc")).unwrap();
    assert_eq!(store.ids().unwrap(), ["a", "b"]);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_symlinked_document_is_never_followed() {
    let dir = temp_dir("link");
    fs::create_dir_all(&dir).unwrap();
    let secret = dir.join("secret.txt");
    fs::write(&secret, "LMDOC1\nformat=text\ntitle=t\n\n秘密").unwrap();
    std::os::unix::fs::symlink(&secret, dir.join("l.doc")).unwrap();
    let store = DocStore::new(&dir);
    assert!(matches!(store.load("l"), Err(WorkspaceError::Corrupt)));
    assert!(store.ids().unwrap().is_empty(), "リンクは一覧に出さない");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn ids_are_lowercase_and_os_reserved_names_are_refused() {
    for bad in [
        "con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9", "A", "Doc",
    ] {
        assert!(
            matches!(check_id(bad), Err(WorkspaceError::InvalidId)),
            "{bad:?}"
        );
    }
    for ok in [
        "console", "com0", "com10", "lpt", "nullable", "doc-1", "a_b",
    ] {
        assert!(check_id(ok).is_ok(), "{ok:?}");
    }
}

#[test]
fn titles_cannot_carry_invisible_or_direction_control_characters() {
    let dir = temp_dir("title2");
    let store = DocStore::new(&dir);
    for bad in [
        "a\u{202E}b",
        "a\u{200B}b",
        "a\u{2028}b",
        "a\u{2029}b",
        "a\u{FEFF}b",
        "a\u{2066}b",
        "a\u{061C}b",
        "a\u{00AD}b",
        "a\u{180E}b",
        "a\u{FFF9}b",
    ] {
        let mut d = doc("t", "x");
        d.title = bad.to_string();
        assert!(
            matches!(store.save(&d), Err(WorkspaceError::InvalidTitle(_))),
            "{bad:?}"
        );
    }
    let mut d = doc("t", "x");
    d.title = "契約書 第12条（解除）".into();
    assert!(store.save(&d).is_ok());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stale_temp_files_are_cleaned_but_other_files_are_not() {
    let dir = temp_dir("tmp");
    let store = DocStore::new(&dir);
    store.save(&doc("keep", "本文")).unwrap();
    let old = SystemTime::now() - Duration::from_secs(600);
    let make = |name: &str, mtime: SystemTime| {
        let f = File::create(dir.join(name)).unwrap();
        f.set_modified(mtime).unwrap();
    };
    make("a.123-4.tmp", old); // 古い一時ファイル: 消す
    make("b.9-0.tmp", SystemTime::now()); // 新しい（書き込み中かもしれない）: 残す
    make("notes.tmp", old); // 形式が違う: 残す
    make("Bad.1-2.tmp", old); // id が不正: 残す
    assert_eq!(
        store.clean_stale_tmp(Duration::ZERO),
        1,
        "60 秒未満は指定できない（0 でも新しいものは消さない）"
    );
    assert!(!dir.join("a.123-4.tmp").exists());
    for kept in ["b.9-0.tmp", "notes.tmp", "Bad.1-2.tmp", "keep.doc"] {
        assert!(dir.join(kept).exists(), "{kept}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stats_count_documents_and_bytes_and_the_version_marker_round_trips() {
    let dir = temp_dir("stats");
    let store = DocStore::new(&dir);
    assert_eq!(store.stats().unwrap(), (0, 0));
    assert_eq!(store.read_version(), None);
    store.save(&doc("a", "12345")).unwrap();
    store.save(&doc("b", "x")).unwrap();
    let (n, bytes) = store.stats().unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        bytes,
        store.byte_len("a").unwrap() + store.byte_len("b").unwrap()
    );
    assert_eq!(store.byte_len("zzz"), None);
    store.write_version(7).unwrap();
    assert_eq!(store.read_version(), Some(7));
    assert_eq!(
        store.ids().unwrap(),
        ["a", "b"],
        "版の記録ファイルは文書に数えない"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn only_exact_temp_names_count_as_temp_files() {
    assert!(is_tmp_name("a.123-4.tmp"));
    assert!(is_tmp_name("a-b_c.9-0.tmp"));
    for bad in [
        "a.tmp",
        "a.123.tmp",
        "a.-4.tmp",
        "a.123-.tmp",
        "a.x-4.tmp",
        "Bad.1-2.tmp",
        "con.1-2.tmp",
        "a.1-2.doc",
        "a.1-2.tmp.bak",
    ] {
        assert!(!is_tmp_name(bad), "{bad}");
    }
}

#[test]
fn byte_len_refuses_ids_that_could_leave_the_directory() {
    let dir = temp_dir("bl");
    let store = DocStore::new(&dir);
    assert_eq!(store.byte_len("../x"), None);
    assert_eq!(store.byte_len("a/b"), None);
}
