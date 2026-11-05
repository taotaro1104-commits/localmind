//! 入力の検証・復元量の上限・ファイル名の安全性・ヘッダの完全性のテスト。

use super::fsio::create_new;
use super::scan::scan_dir_limited;
use super::tests::{chunk, path_of, save, temp_root};
use super::*;

// ---- ヘッダの完全性（版 3 の CRC はヘッダも守る）--------------------------------------------

#[test]
fn a_flipped_bit_in_the_header_is_detected_by_the_checksum() {
    let good = encode(&chunk(0, 50));
    // サンプルレート（8〜12）、開始位置（16〜24）。値は範囲内の別の値に化けるので、範囲検査では見つからない。
    for (range, label) in [(8..12, "sample_rate"), (16..24, "start_ms")] {
        let mut b = good.clone();
        b[range.start] ^= 0x01;
        let err = decode(&b, 0).unwrap_err();
        assert!(
            matches!(&err, IngestError::Corrupt(m) if m.contains("チェックサム")),
            "{label}: {err:?}"
        );
    }
}

#[test]
fn the_format_version_is_three_and_older_files_are_not_misread() {
    let mut b = encode(&chunk(0, 10));
    assert_eq!(&b[4..8], &3u32.to_le_bytes());
    b[4..8].copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(decode(&b, 0), Err(IngestError::Corrupt(_))));
}

// ---- 連番の上限 -----------------------------------------------------------------------------

#[test]
fn a_chunk_index_that_cannot_be_reloaded_is_not_saved() {
    let root = temp_root("index-cap");
    let store = FsChunkStore::new(&root);
    let mut c = chunk(0, 10);
    c.index = MAX_CHUNK_INDEX + 1;
    assert!(matches!(
        store.save_chunk("r", &c),
        Err(IngestError::InvalidFormat(_))
    ));
    assert!(!root.join("r").exists(), "拒否したのにディレクトリができた");

    c.index = MAX_CHUNK_INDEX;
    assert!(validate_chunk(&c).is_ok());
    assert_eq!(
        parse_chunk_file_name(&chunk_file_name(MAX_CHUNK_INDEX)),
        Some(MAX_CHUNK_INDEX)
    );
    let _ = fs::remove_dir_all(&root);
}

// ---- メタ情報の読み込み ---------------------------------------------------------------------

#[test]
fn an_oversized_meta_file_is_rejected() {
    let root = temp_root("meta-size");
    fs::create_dir_all(root.join("r")).unwrap();
    let mut text = String::from("consent_confirmed=true\nsample_rate=16000\nstarted_at_ms=0\n");
    text.push_str(&"#".repeat(MAX_META_BYTES as usize));
    fs::write(root.join("r").join(META_FILE), text).unwrap();
    let err = FsChunkStore::new(&root).load_meta("r").unwrap_err();
    assert!(
        matches!(&err, IngestError::Corrupt(m) if m.contains("大きすぎ")),
        "{err:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_directory_named_like_the_meta_file_is_not_read() {
    let root = temp_root("meta-dir");
    fs::create_dir_all(root.join("r").join(META_FILE)).unwrap();
    assert!(FsChunkStore::new(&root).load_meta("r").is_err());
    let _ = fs::remove_dir_all(&root);
}

// ---- 一時ファイル ---------------------------------------------------------------------------

#[test]
fn temporary_files_are_created_exclusively() {
    let root = temp_root("create-new");
    fs::create_dir_all(&root).unwrap();
    let p = root.join("x.tmp");
    fs::write(&p, b"existing").unwrap();
    let err = create_new(&p).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&p).unwrap(), b"existing", "既存の内容が壊された");
    let _ = fs::remove_dir_all(&root);
}

// ---- 復元量の上限 ---------------------------------------------------------------------------

#[test]
fn recovery_stops_at_the_memory_limit_and_reports_the_rest() {
    let root = temp_root("restore-cap");
    let store = FsChunkStore::new(&root);
    for i in 0..4 {
        save(&store, "r", &chunk(i, 100)).unwrap(); // 1 チャンク = 200 バイト
    }
    let r = store.load_chunks_limited("r", 450, usize::MAX).unwrap();
    assert_eq!(r.chunks.len(), 2, "{:?}", r.skipped);
    assert_eq!(r.skipped.len(), 2);
    assert!(
        r.skipped.iter().all(|s| s.contains("上限")),
        "{:?}",
        r.skipped
    );

    // ちょうど上限に収まるなら全て読める。
    let all = store.load_chunks_limited("r", 800, usize::MAX).unwrap();
    assert_eq!(all.chunks.len(), 4);
    assert!(all.skipped.is_empty());
    let _ = fs::remove_dir_all(&root);
}

// ---- ファイル名の安全性 ---------------------------------------------------------------------

#[test]
fn control_characters_in_file_names_are_escaped_in_reports() {
    assert_eq!(display_name("a\nb\u{1b}[31mc.tmp"), "a\\nb\\u{1b}[31mc.tmp");
    assert_eq!(display_name("plain.tmp"), "plain.tmp");
}

// ---- 構造 -----------------------------------------------------------------------------------

#[test]
fn files_are_found_in_index_order_even_if_the_directory_lists_them_unsorted() {
    let root = temp_root("scan-order");
    let store = FsChunkStore::new(&root);
    for i in [3u32, 0, 2, 1] {
        save(&store, "r", &chunk(i, 10)).unwrap();
    }
    let scan = scan_dir(&root.join("r")).unwrap();
    let indices: Vec<u32> = scan.found.iter().map(|(i, _)| *i).collect();
    assert_eq!(indices, [0, 1, 2, 3]);
    let r = store.load_chunks("r").unwrap();
    assert_eq!(
        r.chunks.iter().map(|c| c.index).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert!(path_of(&root, "r", 0).is_file());
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_symlinked_recording_directory_is_refused_for_writes_and_reads() {
    let root = temp_root("symlink-dir");
    let target = root.join("elsewhere");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&target, root.join("r")).unwrap();
    let store = FsChunkStore::new(&root);
    assert!(save(&store, "r", &chunk(0, 10)).is_err());
    assert!(store.load_chunks("r").is_err());
    assert_eq!(
        fs::read_dir(&target).unwrap().count(),
        0,
        "リンク先に書かれた"
    );
    let _ = fs::remove_dir_all(&root);
}

/// 制御文字や表示を反転させる文字を含む一時ファイル名は、報告文字列ではエスケープされる。
#[test]
fn skipped_reports_escape_unusual_characters_in_file_names() {
    let root = temp_root("escape-report");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 10)).unwrap();
    // U+202E（右から左への上書き）は、多くの OS でファイル名に使え、表示を偽装できる。
    fs::write(root.join("r").join("evil\u{202e}gnp.tmp"), b"x").unwrap();
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1);
    assert_eq!(r.skipped.len(), 1, "{:?}", r.skipped);
    assert!(!r.skipped[0].contains('\u{202e}'), "{:?}", r.skipped);
    assert!(r.skipped[0].contains(r"\u{202e}"), "{:?}", r.skipped);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn recovery_stops_at_the_chunk_count_limit() {
    let root = temp_root("chunk-cap");
    let store = FsChunkStore::new(&root);
    for i in 0..5 {
        save(&store, "r", &chunk(i, 0)).unwrap(); // 空のチャンクはバイト数の上限に掛からない
    }
    let r = store.load_chunks_limited("r", u64::MAX, 3).unwrap();
    assert_eq!(r.chunks.len(), 3);
    assert_eq!(r.skipped.len(), 2, "{:?}", r.skipped);
    assert!(r.skipped.iter().all(|s| s.contains("チャンク数の上限")));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_skipped_report_is_capped() {
    let root = temp_root("skip-cap");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 10)).unwrap();
    for i in 0..(MAX_SKIPPED_REPORTS + 50) {
        fs::write(root.join("r").join(format!("junk{i}.tmp")), b"").unwrap();
    }
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1);
    assert_eq!(r.skipped.len(), MAX_SKIPPED_REPORTS + 1);
    assert!(
        r.skipped.last().unwrap().contains("ほか 50 件"),
        "{:?}",
        r.skipped.last()
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn load_meta_of_a_missing_recording_is_an_error() {
    let root = temp_root("meta-missing");
    assert!(FsChunkStore::new(&root).load_meta("nope").is_err());
}

#[cfg(unix)]
#[test]
fn meta_through_a_symlink_or_a_symlinked_directory_is_refused() {
    let root = temp_root("meta-symlink");
    fs::create_dir_all(root.join("r")).unwrap();
    let real = root.join("real-meta.txt");
    fs::write(
        &real,
        "consent_confirmed=true\nsample_rate=16000\nstarted_at_ms=0\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, root.join("r").join(META_FILE)).unwrap();
    let store = FsChunkStore::new(&root);
    assert!(matches!(store.load_meta("r"), Err(IngestError::Corrupt(_))));

    std::os::unix::fs::symlink(root.join("r"), root.join("linked")).unwrap();
    assert!(matches!(store.load_meta("linked"), Err(IngestError::Io(_))));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn scanning_stops_at_the_entry_limit_and_keeps_what_was_found() {
    let root = temp_root("scan-cap");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 10)).unwrap();
    for i in 0..10 {
        fs::write(root.join("r").join(format!("other{i}.txt")), b"").unwrap();
    }
    let scan = scan_dir_limited(&root.join("r"), 3).unwrap();
    assert!(scan.truncated);
    assert!(scan.skipped.is_empty());
    // 上限が十分なら打ち切られず、何も報告されない。
    let full = scan_dir_limited(&root.join("r"), 100).unwrap();
    assert!(!full.truncated && full.skipped.is_empty());
    // エントリ数がちょうど上限のときは、打ち切りにならない。
    let total = fs::read_dir(root.join("r")).unwrap().count();
    assert!(!scan_dir_limited(&root.join("r"), total).unwrap().truncated);
    assert!(
        scan_dir_limited(&root.join("r"), total - 1)
            .unwrap()
            .truncated
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unrelated_entries_never_stop_the_scan() {
    let root = temp_root("scan-unrelated");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 10)).unwrap();
    fs::create_dir_all(root.join("r").join("subdir")).unwrap();
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    let _ = fs::remove_dir_all(&root);
}
