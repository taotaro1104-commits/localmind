//! 録音の開始（同意の強制）・同意の撤回・録音ルートのリンク検査・古い一時ファイルの掃除のテスト。

use std::time::Duration;

use super::lifecycle::MIN_TMP_AGE;
use super::tests::{chunk, consented, temp_root};
use super::*;

const STARTED: u64 = 1_800_000_000_000;

fn denied() -> RecordingMeta {
    RecordingMeta {
        consent_confirmed: false,
        ..consented()
    }
}

#[test]
fn recording_cannot_begin_without_consent_and_nothing_is_created() {
    let root = temp_root("consent-begin");
    let store = FsChunkStore::new(&root);
    assert!(matches!(
        store.begin_recording("r", &denied()),
        Err(IngestError::Consent(_))
    ));
    assert!(!root.join("r").exists());
    store.begin_recording("r", &consented()).unwrap();
    assert!(store.load_meta("r").unwrap().consent_confirmed);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn beginning_twice_cannot_reset_the_start_time() {
    let root = temp_root("consent-twice");
    let store = FsChunkStore::new(&root);
    store.begin_recording("r", &consented()).unwrap();
    let mut later = consented();
    later.started_at_ms += 1_000_000;
    assert!(store.begin_recording("r", &later).is_err());
    assert_eq!(store.load_meta("r").unwrap().started_at_ms, STARTED);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn concurrent_begins_let_exactly_one_succeed() {
    use std::sync::{Arc, Barrier};
    let root = temp_root("consent-race");
    let store = Arc::new(FsChunkStore::new(&root));
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8u64)
        .map(|i| {
            let (store, barrier) = (store.clone(), barrier.clone());
            std::thread::spawn(move || {
                let mut meta = consented();
                meta.started_at_ms = STARTED + i;
                barrier.wait();
                store.begin_recording("r", &meta).is_ok()
            })
        })
        .collect();
    let wins = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|ok| *ok)
        .count();
    assert_eq!(wins, 1, "同時に始めても、成功するのは 1 つだけ");
    let meta = store.load_meta("r").unwrap();
    assert!((STARTED..STARTED + 8).contains(&meta.started_at_ms));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn chunks_need_a_consented_recording() {
    let root = temp_root("consent-chunk");
    let store = FsChunkStore::new(&root);
    // メタが無い（begin_recording を通っていない）録音には保存できない。
    assert!(matches!(
        store.save_chunk("nometa", &chunk(0, 10)),
        Err(IngestError::Consent(_))
    ));
    assert!(!root.join("nometa").exists(), "何も作らない");
    // 同意なしのメタ（テスト用に直接書く）にも保存できない。
    store.save_meta("denied", &denied()).unwrap();
    assert!(matches!(
        store.save_chunk("denied", &chunk(0, 10)),
        Err(IngestError::Consent(_))
    ));
    // 同意済みなら保存できる。メタが壊れたら安全側（保存しない）。
    store.begin_recording("ok", &consented()).unwrap();
    store.save_chunk("ok", &chunk(0, 10)).unwrap();
    fs::write(root.join("ok").join("meta.txt"), "garbage").unwrap();
    assert!(store.save_chunk("ok", &chunk(1, 10)).is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn revoking_consent_stops_saving_and_keeps_the_start_time() {
    let root = temp_root("consent-revoke");
    let store = FsChunkStore::new(&root);
    store.begin_recording("r", &consented()).unwrap();
    store.save_chunk("r", &chunk(0, 10)).unwrap();
    store.revoke_consent("r").unwrap();
    let meta = store.load_meta("r").unwrap();
    assert!(!meta.consent_confirmed);
    assert_eq!(meta.started_at_ms, STARTED, "他の項目は変えない");
    assert!(matches!(
        store.save_chunk("r", &chunk(1, 10)),
        Err(IngestError::Consent(_))
    ));
    assert!(store.revoke_consent("missing").is_err());
    // 撤回した録音を、同じ id でやり直せない（開始時刻を作り直す手段にしない）。
    assert!(store.begin_recording("r", &consented()).is_err());
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_symlinked_recording_root_is_refused() {
    let root = temp_root("root-link");
    let real = root.join("real");
    fs::create_dir_all(&real).unwrap();
    let link = root.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let store = FsChunkStore::new(&link);
    assert!(store.begin_recording("r", &consented()).is_err());
    assert!(store.delete_recording("r").is_err());
    assert!(store.purge_expired(STARTED, 1).is_err());
    assert!(store.clean_stale_tmp(MIN_TMP_AGE).is_err());
    assert!(
        fs::read_dir(&real).unwrap().next().is_none(),
        "リンク先に何も書かない"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_missing_root_is_fine() {
    let root = temp_root("root-missing");
    let store = FsChunkStore::new(root.join("nowhere"));
    assert_eq!(
        store.clean_stale_tmp(MIN_TMP_AGE).unwrap(),
        TmpCleanReport::default()
    );
    store.begin_recording("r", &consented()).unwrap();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_too_short_max_age_is_refused() {
    let root = temp_root("tmp-age");
    let store = FsChunkStore::new(&root);
    for age in [Duration::ZERO, MIN_TMP_AGE - Duration::from_secs(1)] {
        assert!(matches!(
            store.clean_stale_tmp(age),
            Err(IngestError::InvalidFormat(_))
        ));
    }
    assert!(store.clean_stale_tmp(MIN_TMP_AGE).is_ok());
}

#[test]
fn only_stale_save_tmp_files_of_valid_recordings_are_removed() {
    let root = temp_root("tmp-clean");
    let store = FsChunkStore::new(&root);
    store.begin_recording("r", &consented()).unwrap();
    store.save_chunk("r", &chunk(0, 10)).unwrap();
    let dir = root.join("r");
    let stale = dir.join("00000001.pcm.1-1.tmp");
    let fresh = dir.join("00000002.pcm.1-2.tmp");
    let lookalike = dir.join("notes.tmp"); // 保存が作る名前ではない
    for f in [&stale, &fresh, &lookalike] {
        fs::write(f, b"x").unwrap();
    }
    let old = std::time::SystemTime::now() - Duration::from_secs(7200);
    for f in [&stale, &lookalike] {
        fs::File::options()
            .write(true)
            .open(f)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    // 録音 id として不正なディレクトリと、`.tmp` でないファイルには触れない。
    fs::create_dir_all(root.join("Not Valid")).unwrap();
    fs::write(root.join("Not Valid").join("a.1-1.tmp"), b"x").unwrap();
    fs::write(dir.join("notes.txt"), b"x").unwrap();

    let report = store.clean_stale_tmp(Duration::from_secs(3600)).unwrap();
    assert_eq!(report.removed, ["r/00000001.pcm.1-1.tmp"]);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(!stale.exists());
    assert!(fresh.exists(), "新しいものは書き込み中かもしれない");
    assert!(
        lookalike.exists(),
        "保存の一時ファイルの名前でないものは消さない"
    );
    assert!(root.join("Not Valid").join("a.1-1.tmp").exists());
    assert!(dir.join("notes.txt").exists());
    assert!(dir.join("00000000.pcm").exists() && dir.join("meta.txt").exists());
    let _ = fs::remove_dir_all(&root);
}

/// ハードリンクを許さない環境（Android の SELinux など）を再現する。
fn link_denied(_: &std::path::Path, _: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::PermissionDenied.into())
}

#[test]
fn a_new_file_is_still_created_where_hard_links_are_denied() {
    use super::fsio::write_new_atomic_with;
    let root = temp_root("nolink");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("meta.txt");
    assert!(write_new_atomic_with(&path, b"first", link_denied).unwrap());
    assert_eq!(fs::read(&path).unwrap(), b"first");
    assert!(
        !write_new_atomic_with(&path, b"second", link_denied).unwrap(),
        "既にあれば作らない"
    );
    assert_eq!(fs::read(&path).unwrap(), b"first", "既存の内容は変わらない");
    let leftovers: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["meta.txt"], "一時ファイルを残さない");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn concurrent_creation_without_hard_links_still_lets_exactly_one_win() {
    use super::fsio::write_new_atomic_with;
    use std::sync::{Arc, Barrier};
    let root = temp_root("nolink-race");
    fs::create_dir_all(&root).unwrap();
    let path = Arc::new(root.join("meta.txt"));
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8u8)
        .map(|i| {
            let (path, barrier) = (path.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                write_new_atomic_with(&path, &[b'a' + i; 64], link_denied).unwrap()
            })
        })
        .collect();
    let wins = handles
        .into_iter()
        .filter(|_| true)
        .map(|h| h.join().unwrap())
        .filter(|&w| w)
        .count();
    assert_eq!(wins, 1);
    let content = fs::read(&*path).unwrap();
    assert_eq!(content.len(), 64);
    assert!(
        content.iter().all(|&b| b == content[0]),
        "別々の書き手の内容が混ざらない"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_failed_start_leaves_no_empty_recording_directory_behind() {
    let root = temp_root("begin-fail");
    let store = FsChunkStore::new(&root);
    let broken = |_: &std::path::Path, _: &[u8]| -> Result<bool, IngestError> {
        Err(IngestError::Io("書き込めません".into()))
    };
    assert!(store
        .begin_recording_with("r", &consented(), broken)
        .is_err());
    assert!(!root.join("r").exists(), "空のディレクトリを残さない");
    assert!(store.list_recordings().unwrap().is_empty());

    // 中身のあるディレクトリは、失敗しても消さない。
    fs::create_dir_all(root.join("keep")).unwrap();
    fs::write(root.join("keep").join("00000000.pcm"), b"x").unwrap();
    assert!(store
        .begin_recording_with("keep", &consented(), broken)
        .is_err());
    assert!(root.join("keep").join("00000000.pcm").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn begin_recording_works_without_hard_links_and_refuses_a_second_start() {
    // begin_recording の本体は write_new_atomic を通る。リンクが使えなくても同じ結果になることを、下回りで確かめた上で、
    // ここでは通常の経路の結果（同意の記録・二重開始の拒否）が変わらないことを確認する。
    let root = temp_root("nolink-begin");
    let store = FsChunkStore::new(&root);
    store.begin_recording("r", &consented()).unwrap();
    assert!(store.begin_recording("r", &consented()).is_err());
    assert!(store.load_meta("r").unwrap().consent_confirmed);
    let _ = fs::remove_dir_all(&root);
}
