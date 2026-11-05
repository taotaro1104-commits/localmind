//! チャンクファイルのデコード検証・メタ情報・削除のテスト。

use super::tests::{chunk, path_of, save, temp_root};
use super::*;
use crate::audio::MAX_SAMPLE_RATE;

// ---- デコードの検証 -------------------------------------------------------------------

/// ヘッダが巨大なサンプル数を主張しても、実サイズと一致しなければ確保せず拒否する。
#[test]
fn a_header_claiming_a_huge_sample_count_is_rejected_without_allocating() {
    let mut bytes = encode(&chunk(0, 10));
    bytes[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(decode(&bytes, 0), Err(IngestError::Corrupt(_))));

    let max_plus = u64::from(1000u32) * u64::from(MAX_CHUNK_SECONDS) + 1;
    bytes[24..32].copy_from_slice(&max_plus.to_le_bytes());
    assert!(matches!(decode(&bytes, 0), Err(IngestError::Corrupt(_))));
}

#[test]
fn an_oversized_file_is_rejected_before_reading() {
    let root = temp_root("oversized");
    fs::create_dir_all(&root).unwrap();
    let p = root.join("00000000.pcm");
    let f = fs::File::create(&p).unwrap();
    f.set_len(MAX_FILE_BYTES + 1).unwrap(); // スパースファイル。読み込まれたら遅い／失敗する
    drop(f);
    let err = read_chunk(&p, 0).unwrap_err();
    assert!(
        matches!(&err, IngestError::Corrupt(m) if m.contains("大きすぎ")),
        "{err:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn bad_headers_are_rejected_individually() {
    let good = encode(&chunk(0, 10));
    assert!(decode(&good, 0).is_ok());

    let mut b = good.clone();
    b[0] = b'X';
    assert!(decode(&b, 0).is_err(), "magic");

    let mut b = good.clone();
    b[4] = 9;
    assert!(decode(&b, 0).is_err(), "version");

    let mut b = good.clone();
    b[8..12].copy_from_slice(&0u32.to_le_bytes());
    assert!(decode(&b, 0).is_err(), "sample rate 0");

    assert!(decode(&good[..HEADER_LEN - 1], 0).is_err(), "short header");
    assert!(decode(&good, 1).is_err(), "index mismatch");
    let mut b = good.clone();
    b.pop();
    assert!(decode(&b, 0).is_err(), "odd body length");
}

/// サンプルレートが上限を超えるヘッダは、サンプル数が小さくても拒否する（`max_count` 経由ではなく独立に）。
#[test]
fn a_sample_rate_above_the_limit_is_rejected_on_its_own() {
    let mut b = encode(&chunk(0, 10));
    b[8..12].copy_from_slice(&(MAX_SAMPLE_RATE + 1).to_le_bytes());
    assert!(matches!(decode(&b, 0), Err(IngestError::Corrupt(_))));
}

#[test]
fn a_start_position_beyond_the_limit_is_rejected() {
    let mut b = encode(&chunk(0, 10));
    b[16..24].copy_from_slice(&(MAX_START_MS + 1).to_le_bytes());
    assert!(matches!(decode(&b, 0), Err(IngestError::Corrupt(_))));
    b[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(decode(&b, 0).is_err());
}

#[test]
fn extra_trailing_bytes_are_rejected() {
    let mut b = encode(&chunk(0, 10));
    b.extend_from_slice(&[0, 0]); // 偶数バイトの余り（サンプル数と本体の長さが食い違う）
    assert!(matches!(decode(&b, 0), Err(IngestError::Corrupt(_))));
}

#[test]
fn a_flipped_bit_in_the_audio_is_detected_by_the_checksum() {
    let mut b = encode(&chunk(0, 50));
    let last = b.len() - 1;
    b[last] ^= 0x01;
    let err = decode(&b, 0).unwrap_err();
    assert!(
        matches!(&err, IngestError::Corrupt(m) if m.contains("チェックサム")),
        "{err:?}"
    );

    // 保存先のファイルでも、復元時に壊れたチャンクとして扱われる。
    let root = temp_root("bitrot");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 50)).unwrap();
    let p = path_of(&root, "r", 0);
    let mut bytes = fs::read(&p).unwrap();
    let mid = HEADER_LEN + 10;
    bytes[mid] ^= 0x80;
    fs::write(&p, bytes).unwrap();
    let r = store.load_chunks("r").unwrap();
    assert!(r.chunks.is_empty());
    assert!(r.skipped[0].contains("チェックサム"), "{:?}", r.skipped);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn crc32_matches_the_standard_check_value() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

// ---- メタ情報 -------------------------------------------------------------------------

#[test]
fn meta_round_trips_and_records_consent() {
    let root = temp_root("meta");
    let store = FsChunkStore::new(&root);
    let meta = RecordingMeta {
        consent_confirmed: true,
        sample_rate: 16_000,
        started_at_ms: 1_730_000_000_000,
    };
    store.save_meta("r", &meta).unwrap();
    assert_eq!(store.load_meta("r").unwrap(), meta);
    let unconfirmed = RecordingMeta {
        consent_confirmed: false,
        ..meta
    };
    store.save_meta("r", &unconfirmed).unwrap();
    assert!(!store.load_meta("r").unwrap().consent_confirmed);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn missing_or_malformed_meta_is_an_error() {
    let root = temp_root("badmeta");
    let store = FsChunkStore::new(&root);
    assert!(store.load_meta("r").is_err()); // ファイル無し
    fs::create_dir_all(root.join("r")).unwrap();
    fs::write(root.join("r").join("meta.txt"), "sample_rate=16000\n").unwrap();
    assert!(matches!(store.load_meta("r"), Err(IngestError::Corrupt(_))));
    fs::write(
        root.join("r").join("meta.txt"),
        "consent_confirmed=maybe\nsample_rate=1\nstarted_at_ms=1\n",
    )
    .unwrap();
    assert!(matches!(store.load_meta("r"), Err(IngestError::Corrupt(_))));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn meta_sample_rate_is_validated_on_save_and_on_load() {
    let root = temp_root("meta-rate");
    let store = FsChunkStore::new(&root);
    let bad = RecordingMeta {
        consent_confirmed: true,
        sample_rate: 0,
        started_at_ms: 0,
    };
    assert!(matches!(
        store.save_meta("r", &bad),
        Err(IngestError::InvalidFormat(_))
    ));
    assert!(!root.join("r").exists());

    fs::create_dir_all(root.join("r")).unwrap();
    for rate in ["0", &(MAX_SAMPLE_RATE + 1).to_string()] {
        fs::write(
            root.join("r").join("meta.txt"),
            format!("consent_confirmed=true\nsample_rate={rate}\nstarted_at_ms=1\n"),
        )
        .unwrap();
        assert!(
            matches!(store.load_meta("r"), Err(IngestError::Corrupt(_))),
            "rate {rate}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

// ---- 削除 -----------------------------------------------------------------------------

#[test]
fn delete_removes_the_whole_recording() {
    let root = temp_root("delete");
    let store = FsChunkStore::new(&root);
    save(&store, "r", &chunk(0, 10)).unwrap();
    store
        .save_meta(
            "r",
            &RecordingMeta {
                consent_confirmed: true,
                sample_rate: 1000,
                started_at_ms: 0,
            },
        )
        .unwrap();
    assert!(store.delete_recording("r").unwrap());
    assert!(!store.delete_recording("r").unwrap());
    assert!(store.load_chunks("r").unwrap().chunks.is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn delete_refuses_a_recording_path_that_is_not_a_plain_directory() {
    let root = temp_root("delete-file");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("r"), b"a file, not a directory").unwrap();
    let store = FsChunkStore::new(&root);
    assert!(matches!(
        store.delete_recording("r"),
        Err(IngestError::Io(_))
    ));
    assert!(root.join("r").exists(), "拒否したら何も消さない");
    let _ = fs::remove_dir_all(&root);
}

/// シンボリックリンクの録音ディレクトリは、リンク先を消さないよう拒否する（作れない環境では省略）。
#[test]
fn delete_never_follows_a_symlinked_recording_directory() {
    let base = temp_root("delete-link");
    let target = base.join("precious");
    let root = base.join("root");
    fs::create_dir_all(&target).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(target.join("keep.txt"), b"keep").unwrap();

    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&target, root.join("r")).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&target, root.join("r")).is_ok();
    #[cfg(not(any(unix, windows)))]
    let linked = false;
    if !linked {
        let _ = fs::remove_dir_all(&base);
        return; // 権限が無く、リンクを作れない環境
    }

    let store = FsChunkStore::new(&root);
    assert!(store.delete_recording("r").is_err());
    assert!(target.join("keep.txt").exists(), "リンク先は消えない");
    let _ = fs::remove_dir_all(&base);
}
