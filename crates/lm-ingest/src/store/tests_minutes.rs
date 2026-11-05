//! 議事録の保存・読み込みのテスト。

use super::tests::temp_root;
use super::*;
use crate::minutes::{ActionItem, Minutes, MinutesItem};

fn sample() -> Minutes {
    Minutes {
        summary: vec![MinutesItem {
            text: "予算案を承認した".into(),
            citations: vec![0],
        }],
        decisions: vec![MinutesItem {
            text: "承認".into(),
            citations: vec![0, 2000],
        }],
        action_items: vec![
            ActionItem {
                text: "見積もりを出す".into(),
                assignee: Some("鈴木".into()),
                due: Some("来週".into()),
                citations: vec![2000],
                done: true,
            },
            ActionItem {
                text: "担当未定の件".into(),
                assignee: None,
                due: None,
                citations: vec![4000],
                done: false,
            },
        ],
        open_issues: vec![MinutesItem {
            text: "納期が未定".into(),
            citations: vec![4000],
        }],
        rejected: 3,
    }
}

fn write_raw(root: &std::path::Path, id: &str, text: &str) {
    fs::create_dir_all(root.join(id)).unwrap();
    fs::write(root.join(id).join("minutes.txt"), text).unwrap();
}

#[test]
fn minutes_round_trip_including_done_flags_and_missing_fields() {
    let root = temp_root("minutes-rt");
    let store = FsChunkStore::new(&root);
    assert_eq!(store.load_minutes("r").unwrap(), None, "未保存は None");
    store.save_minutes("r", &sample()).unwrap();
    assert_eq!(store.load_minutes("r").unwrap(), Some(sample()));

    // 完了の切り替えを保存し直せる。
    let mut m = sample();
    assert!(m.set_done(1, true));
    store.save_minutes("r", &m).unwrap();
    assert!(store.load_minutes("r").unwrap().unwrap().action_items[1].done);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn malformed_minutes_files_are_rejected_not_trusted() {
    let root = temp_root("minutes-bad");
    let store = FsChunkStore::new(&root);
    let header = "LMMIN 1\nrejected\t0\n";
    let cases = [
        "LMMIN 2\nrejected\t0\n".to_string(),
        "garbage".to_string(),
        format!("{header}X\t0\t本文\n"),
        format!("{header}S\t0\n"),
        format!("{header}S\t\t本文\n"),
        format!("{header}S\t5,3\t本文\n"),
        format!("{header}S\t0\t \n"),
        format!("{header}S\t0\t本文\x1b[2J\n"),
        format!("{header}A\t2\t0\t\t\t本文\n"),
        format!("{header}A\t0\t0\t本文\n"),
        format!(
            "{header}S\t{}\t本文\n",
            (0..11).map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        ),
        format!(
            "{header}S\t0\t{}\n",
            "あ".repeat(crate::minutes::MAX_ITEM_CHARS + 1)
        ),
    ];
    for (i, text) in cases.iter().enumerate() {
        write_raw(&root, "r", text);
        assert!(
            matches!(store.load_minutes("r"), Err(IngestError::Corrupt(_))),
            "case {i}: {text:?}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn too_many_items_are_rejected_on_load() {
    let root = temp_root("minutes-many");
    let store = FsChunkStore::new(&root);
    let mut text = String::from("LMMIN 1\nrejected\t0\n");
    for _ in 0..=crate::minutes::MAX_ITEMS_PER_SECTION {
        text.push_str("S\t0\t項目\n");
    }
    write_raw(&root, "r", &text);
    assert!(store.load_minutes("r").is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_oversized_minutes_file_is_rejected() {
    let root = temp_root("minutes-big");
    let store = FsChunkStore::new(&root);
    write_raw(&root, "r", &"#".repeat(5 * 1024 * 1024));
    assert!(store.load_minutes("r").is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn minutes_follow_the_recording_id_rules_and_are_deleted_with_it() {
    let root = temp_root("minutes-id");
    let store = FsChunkStore::new(&root);
    assert!(store.save_minutes("../x", &sample()).is_err());
    assert!(store.load_minutes("../x").is_err());
    store.save_minutes("r", &sample()).unwrap();
    assert!(store.delete_recording("r").unwrap());
    assert_eq!(store.load_minutes("r").unwrap(), None);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn invisible_and_line_separator_characters_are_rejected_on_load() {
    let root = temp_root("minutes-invisible");
    let store = FsChunkStore::new(&root);
    let header = "LMMIN 1\nrejected\t0\n";
    for bad in ["\u{202E}", "\u{200B}", "\u{2028}", "\u{FEFF}"] {
        write_raw(&root, "r", &format!("{header}S\t0\t本文{bad}\n"));
        assert!(store.load_minutes("r").is_err(), "{bad:?}");
        write_raw(&root, "r", &format!("{header}A\t0\t0\t担当{bad}\t\t本文\n"));
        assert!(store.load_minutes("r").is_err(), "assignee {bad:?}");
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn hand_built_invalid_minutes_are_refused_at_save_time() {
    let root = temp_root("minutes-save-bad");
    let store = FsChunkStore::new(&root);
    let item = |text: &str, citations: Vec<u64>| MinutesItem {
        text: text.into(),
        citations,
    };
    let forged = Minutes {
        summary: vec![item("本文\nA\t1\t0\t\t\t偽造", vec![0])],
        ..Minutes::default()
    };
    assert!(store.save_minutes("r", &forged).is_err(), "行の偽造");
    let no_cite = Minutes {
        summary: vec![item("本文", vec![])],
        ..Minutes::default()
    };
    assert!(store.save_minutes("r", &no_cite).is_err(), "引用なし");
    let too_many = Minutes {
        summary: (0..=crate::minutes::MAX_ITEMS_PER_SECTION)
            .map(|_| item("項目", vec![0]))
            .collect(),
        ..Minutes::default()
    };
    assert!(store.save_minutes("r", &too_many).is_err(), "件数超過");
    assert_eq!(store.load_minutes("r").unwrap(), None, "何も書かれていない");
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_is_an_error_not_unsaved() {
    let root = temp_root("minutes-dangling");
    let store = FsChunkStore::new(&root);
    fs::create_dir_all(root.join("r")).unwrap();
    std::os::unix::fs::symlink("/nonexistent/target", root.join("r").join("minutes.txt")).unwrap();
    assert!(store.load_minutes("r").is_err());
    let _ = fs::remove_dir_all(&root);
}
