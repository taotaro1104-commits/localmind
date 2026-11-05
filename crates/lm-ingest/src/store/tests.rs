use super::*;

pub(super) fn temp_root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("lm-ingest-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    p
}

pub(super) fn chunk(index: u32, n: usize) -> Chunk {
    Chunk {
        index,
        start_ms: u64::from(index) * 1000,
        sample_rate: 1000,
        samples: (0..n)
            .map(|i| (i as i16).wrapping_mul(7).wrapping_sub(100))
            .collect(),
    }
}

/// テスト用の「同意済み」メタ。
pub(super) fn consented() -> RecordingMeta {
    RecordingMeta {
        consent_confirmed: true,
        sample_rate: 1000,
        started_at_ms: 1_800_000_000_000,
    }
}

/// テスト用: 録音が未登録なら `begin_recording`（同意済み）してから、チャンクを保存する。
pub(super) fn save(store: &FsChunkStore, id: &str, chunk: &Chunk) -> Result<(), IngestError> {
    if !store.dir(id)?.join("meta.txt").exists() {
        store.begin_recording(id, &consented())?;
    }
    store.save_chunk(id, chunk)
}

pub(super) fn path_of(root: &Path, id: &str, index: u32) -> PathBuf {
    root.join(id).join(chunk_file_name(index))
}

// ---- 保存と復元 ----------------------------------------------------------------------

#[test]
fn chunks_round_trip_exactly() {
    let root = temp_root("roundtrip");
    let store = FsChunkStore::new(&root);
    let chunks = [chunk(0, 1000), chunk(1, 1000), chunk(2, 345)];
    for c in &chunks {
        save(&store, "rec1", c).unwrap();
    }
    let r = store.load_chunks("rec1").unwrap();
    assert_eq!(r.chunks, chunks);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn saving_the_same_index_overwrites() {
    let root = temp_root("overwrite");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 10)).unwrap();
    let newer = chunk(0, 20);
    save(&store, "r", &newer).unwrap();
    assert_eq!(store.load_chunks("r").unwrap().chunks, vec![newer]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unknown_recording_loads_as_empty() {
    let root = temp_root("unknown");
    let r = FsChunkStore::new(&root).load_chunks("nothing").unwrap();
    assert!(r.chunks.is_empty() && r.skipped.is_empty());
}

/// クラッシュ想定: 末尾のチャンクが途中で切れていても、その手前までは復元できる。
#[test]
fn a_truncated_last_chunk_is_dropped_but_earlier_ones_survive() {
    let root = temp_root("truncated");
    let store = FsChunkStore::new(&root);
    for i in 0..3 {
        save(&store, "r", &chunk(i, 500)).unwrap();
    }
    let p = path_of(&root, "r", 2);
    let bytes = fs::read(&p).unwrap();
    fs::write(&p, &bytes[..bytes.len() - 100]).unwrap();

    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 2);
    assert_eq!(r.chunks[1].index, 1);
    assert_eq!(r.skipped.len(), 1);
    assert!(r.skipped[0].starts_with("00000002.pcm"), "{:?}", r.skipped);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_gap_stops_recovery_and_later_chunks_are_reported_not_used() {
    let root = temp_root("gap");
    let store = FsChunkStore::new(&root);
    for i in 0..4 {
        save(&store, "r", &chunk(i, 100)).unwrap();
    }
    fs::remove_file(path_of(&root, "r", 1)).unwrap();
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1);
    assert_eq!(r.chunks[0].index, 0);
    assert_eq!(r.skipped.len(), 2, "{:?}", r.skipped);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_corrupt_middle_chunk_stops_recovery_there() {
    let root = temp_root("corrupt-mid");
    let store = FsChunkStore::new(&root);
    for i in 0..3 {
        save(&store, "r", &chunk(i, 100)).unwrap();
    }
    fs::write(path_of(&root, "r", 1), b"garbage").unwrap();
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1);
    assert_eq!(r.skipped.len(), 2, "{:?}", r.skipped);
    assert!(
        r.skipped[1].contains("手前のチャンクが読めない"),
        "{:?}",
        r.skipped
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn leftover_temp_files_are_ignored_and_reported() {
    let root = temp_root("tmp");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 100)).unwrap();
    // 書き込み中にクラッシュした 2 つ目のチャンク。
    fs::write(root.join("r").join("00000001.pcm.123-0.tmp"), b"partial").unwrap();
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1);
    assert!(
        r.skipped.iter().any(|s| s.contains("一時ファイル")),
        "{:?}",
        r.skipped
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unrelated_files_in_the_directory_are_ignored() {
    let root = temp_root("unrelated");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 100)).unwrap();
    fs::write(root.join("r").join("notes.txt"), b"x").unwrap();
    fs::write(root.join("r").join("0.pcm"), b"x").unwrap(); // 8 桁でない
    fs::write(root.join("r").join("abcdefgh.pcm"), b"x").unwrap();
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_chunk_file_renamed_to_another_index_is_rejected() {
    let root = temp_root("renamed");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 100)).unwrap();
    save(&store, "r", &chunk(1, 100)).unwrap();
    fs::copy(path_of(&root, "r", 1), path_of(&root, "r", 0)).unwrap();
    let r = store.load_chunks("r").unwrap();
    assert!(r.chunks.is_empty());
    assert!(
        r.skipped[0].contains("連番が一致しません"),
        "{:?}",
        r.skipped
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_directory_named_like_a_chunk_is_not_read() {
    let root = temp_root("dir-as-chunk");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 100)).unwrap();
    fs::create_dir_all(path_of(&root, "r", 1)).unwrap();
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1);
    assert!(
        r.skipped
            .iter()
            .any(|s| s.contains("通常のファイルではない")),
        "{:?}",
        r.skipped
    );
    let _ = fs::remove_dir_all(&root);
}

// ---- 復元計画（欠け・順序）-----------------------------------------------------------

#[test]
fn recovery_plan_uses_the_gap_free_prefix_in_index_order() {
    // ファイルシステムの列挙順に依存しない: 順不同の入力でも先頭から連続した範囲を使う。
    let (usable, unusable) = plan_recovery(&[2, 0, 1, 4, 5]);
    assert_eq!(usable, vec![0, 1, 2]);
    assert_eq!(unusable.len(), 2);
    assert_eq!(unusable[0].0, 4);
    assert!(
        unusable[0].1.contains("連番 3 が欠けている"),
        "{unusable:?}"
    );
    assert!(
        unusable[1].1.contains("手前のチャンクが欠けている"),
        "{unusable:?}"
    );
}

#[test]
fn recovery_plan_edge_cases() {
    assert_eq!(plan_recovery(&[]), (vec![], vec![]));
    assert_eq!(plan_recovery(&[0]), (vec![0], vec![]));
    let (usable, unusable) = plan_recovery(&[1, 2]); // 0 が無い
    assert!(usable.is_empty());
    assert_eq!(unusable.len(), 2);
    assert!(unusable[0].1.contains("連番 0 が欠けている"));
    assert_eq!(plan_recovery(&[3, 2, 1, 0]).0, vec![0, 1, 2, 3]);
}

// ---- 録音 id -------------------------------------------------------------------------

#[test]
fn recording_ids_that_could_escape_the_directory_are_rejected() {
    let store = FsChunkStore::new(temp_root("ids"));
    for bad in [
        "",
        "..",
        "../x",
        "a/b",
        "a\\b",
        "a.b",
        ".hidden",
        "C:",
        "a b",
        "日本語",
        "x\0y",
        &"a".repeat(MAX_ID_LEN + 1),
    ] {
        assert!(
            matches!(
                save(&store, bad, &chunk(0, 1)),
                Err(IngestError::InvalidId(_))
            ),
            "{bad:?}"
        );
        assert!(store.load_chunks(bad).is_err(), "{bad:?}");
        assert!(store.delete_recording(bad).is_err(), "{bad:?}");
        assert!(store.load_meta(bad).is_err(), "{bad:?}");
    }
    for ok in [
        "rec1",
        "a-b_c",
        "a",
        "com0",
        "com10",
        "console",
        &"a".repeat(MAX_ID_LEN),
    ] {
        assert!(validate_recording_id(ok).is_ok(), "{ok:?}");
    }
}

/// 大文字小文字を区別しないファイルシステム（Windows・macOS）で別の録音が衝突しないよう、小文字のみ許す。
#[test]
fn uppercase_ids_are_rejected_to_avoid_case_insensitive_collisions() {
    for bad in ["Abc", "ABC", "aBc", "REC1"] {
        assert!(
            matches!(validate_recording_id(bad), Err(IngestError::InvalidId(_))),
            "{bad}"
        );
    }
}

#[test]
fn os_reserved_device_names_are_rejected() {
    for bad in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
        assert!(
            matches!(validate_recording_id(bad), Err(IngestError::InvalidId(_))),
            "{bad}"
        );
    }
}

#[test]
fn invalid_ids_never_touch_the_filesystem() {
    // 専用の親ディレクトリの下に root を作り、「root の外（親）」に何も作られないことを確認する。
    let base = temp_root("noop");
    let root = base.join("root");
    let store = FsChunkStore::new(&root);
    let _ = save(&store, "../escape", &chunk(0, 1));
    assert!(!root.exists());
    assert!(!base.join("escape").exists());
    assert!(!base.exists(), "何も作られていないはず");
}

// ---- 保存前の検証（再読込できないチャンクは保存しない）-------------------------------

#[test]
fn chunks_that_could_not_be_read_back_are_refused_at_save_time() {
    let root = temp_root("refuse");
    let store = FsChunkStore::new(&root);
    let mut zero_rate = chunk(0, 10);
    zero_rate.sample_rate = 0;
    let mut too_long = chunk(0, 10);
    too_long.sample_rate = 1;
    too_long.samples = vec![0; MAX_CHUNK_SECONDS as usize + 1];
    let mut far_future = chunk(0, 10);
    far_future.start_ms = MAX_START_MS + 1;
    for (name, c) in [
        ("rate 0", zero_rate),
        ("too long", too_long),
        ("far future", far_future),
    ] {
        assert!(
            matches!(
                store.save_chunk("r", &c),
                Err(IngestError::InvalidFormat(_))
            ),
            "{name}"
        );
    }
    assert!(store.load_chunks("r").unwrap().chunks.is_empty());
    assert!(!root.join("r").exists(), "拒否したら何も作らない");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn anything_that_passes_validation_round_trips() {
    let root = temp_root("boundary");
    let store = FsChunkStore::new(&root);
    let mut at_limit = chunk(0, 1);
    at_limit.start_ms = MAX_START_MS;
    save(&store, "r", &at_limit).unwrap();
    let mut max_len = chunk(1, 0);
    max_len.sample_rate = 1;
    max_len.start_ms = 0;
    max_len.samples = vec![5; MAX_CHUNK_SECONDS as usize];
    save(&store, "r", &max_len).unwrap();
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks, vec![at_limit, max_len]);
    let _ = fs::remove_dir_all(&root);
}
