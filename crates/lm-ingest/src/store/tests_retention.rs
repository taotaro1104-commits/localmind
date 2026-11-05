//! 自動削除期限のテスト。

use super::retention::{is_expired, MAX_DELETIONS_PER_RUN, MIN_PLAUSIBLE_MS};
use super::tests::{chunk, save, temp_root};
use super::*;

const DAY: u64 = 24 * 60 * 60 * 1000;
/// 現実的な時刻（2027 年ごろ）。`MIN_PLAUSIBLE_MS` より後。
const BASE: u64 = 1_800_000_000_000;

fn record(store: &FsChunkStore, id: &str, started_at_ms: u64) {
    save(store, id, &chunk(0, 10)).unwrap();
    store
        .save_meta(
            id,
            &RecordingMeta {
                consent_confirmed: true,
                sample_rate: 16_000,
                started_at_ms,
            },
        )
        .unwrap();
}

#[test]
fn expiry_is_judged_from_the_start_time() {
    assert!(!is_expired(0, 30 * DAY - 1, 30));
    assert!(is_expired(0, 30 * DAY, 30), "ちょうど期限は切れ");
    assert!(
        !is_expired(5 * DAY, 4 * DAY, 1),
        "時計が巻き戻っていても消さない"
    );
}

#[test]
fn only_expired_recordings_are_deleted() {
    let root = temp_root("purge");
    let store = FsChunkStore::new(&root);
    record(&store, "old", BASE);
    record(&store, "recent", BASE + 29 * DAY);
    let r = store.purge_expired(BASE + 31 * DAY, 30).unwrap();
    assert_eq!(r.deleted, ["old"]);
    assert!(r.kept_with_reason.is_empty(), "{:?}", r.kept_with_reason);
    assert!(!root.join("old").exists());
    assert!(root.join("recent").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn zero_retention_days_is_refused_and_deletes_nothing() {
    let root = temp_root("purge-zero");
    let store = FsChunkStore::new(&root);
    record(&store, "r", BASE);
    assert!(matches!(
        store.purge_expired(BASE + 10 * DAY, 0),
        Err(IngestError::InvalidFormat(_))
    ));
    assert!(root.join("r").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_implausible_now_deletes_nothing() {
    let root = temp_root("purge-now");
    let store = FsChunkStore::new(&root);
    record(&store, "r", BASE);
    for now in [0, 1_800_000_000, MIN_PLAUSIBLE_MS - 1] {
        assert!(
            matches!(
                store.purge_expired(now, 1),
                Err(IngestError::InvalidFormat(_))
            ),
            "now={now}"
        );
    }
    assert!(root.join("r").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn an_implausible_start_time_is_kept_and_reported() {
    let root = temp_root("purge-start");
    let store = FsChunkStore::new(&root);
    record(&store, "zero", 0);
    let r = store.purge_expired(BASE + 1000 * DAY, 1).unwrap();
    assert!(r.deleted.is_empty());
    assert_eq!(r.kept_with_reason.len(), 1, "{:?}", r.kept_with_reason);
    assert!(root.join("zero").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn deletions_per_run_are_capped_and_the_rest_wait_for_next_time() {
    let root = temp_root("purge-cap");
    let store = FsChunkStore::new(&root);
    for i in 0..=MAX_DELETIONS_PER_RUN {
        record(&store, &format!("r{i:04}"), BASE);
    }
    let now = BASE + 10 * DAY;
    let r = store.purge_expired(now, 1).unwrap();
    assert_eq!(r.deleted.len(), MAX_DELETIONS_PER_RUN);
    assert_eq!(r.kept_with_reason.len(), 1, "{:?}", r.kept_with_reason);
    let r2 = store.purge_expired(now, 1).unwrap();
    assert_eq!(r2.deleted.len(), 1, "残りは次回に消える");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn recordings_without_readable_meta_are_kept_and_reported() {
    let root = temp_root("purge-nometa");
    let store = FsChunkStore::new(&root);
    // メタなしの録音（チャンクだけがある）を直接作る。
    fs::create_dir_all(root.join("nometa")).unwrap();
    fs::write(
        root.join("nometa").join(chunk_file_name(0)),
        encode(&chunk(0, 10)),
    )
    .unwrap();
    let r = store.purge_expired(BASE + 1000 * DAY, 1).unwrap();
    assert!(r.deleted.is_empty());
    assert_eq!(r.kept_with_reason.len(), 1, "{:?}", r.kept_with_reason);
    assert!(root.join("nometa").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unrelated_entries_and_a_missing_root_are_left_alone() {
    let root = temp_root("purge-other");
    fs::create_dir_all(root.join("Not Valid ID")).unwrap();
    fs::write(root.join("file.txt"), b"x").unwrap();
    let store = FsChunkStore::new(&root);
    let r = store.purge_expired(BASE + 1000 * DAY, 1).unwrap();
    assert_eq!(r, PurgeReport::default());
    assert!(root.join("Not Valid ID").exists());
    assert!(root.join("file.txt").exists());
    let _ = fs::remove_dir_all(&root);

    let missing = FsChunkStore::new(root.join("nowhere"));
    assert_eq!(
        missing.purge_expired(BASE + DAY, 1).unwrap(),
        PurgeReport::default()
    );
}

#[test]
fn deleting_a_recording_removes_everything_including_the_meta() {
    let root = temp_root("purge-all");
    let store = FsChunkStore::new(&root);
    record(&store, "r", BASE);
    fs::create_dir_all(root.join("r").join("sub")).unwrap();
    fs::write(root.join("r").join("sub").join("x"), b"x").unwrap();
    assert!(store.delete_recording("r").unwrap());
    assert!(!root.join("r").exists());
    assert!(!store.delete_recording("r").unwrap());
    let _ = fs::remove_dir_all(&root);
}

/// 削除が途中で失敗しても、メタは残る（次回の期限切れ判定に使える）。
/// 読み取り専用のディレクトリでは削除が失敗する（root 権限では失敗しないので、その場合は何もしない）。
#[cfg(unix)]
#[test]
fn a_failed_delete_leaves_the_meta_so_the_recording_can_be_purged_later() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("purge-partial");
    let store = FsChunkStore::new(&root);
    record(&store, "r", BASE);
    let dir = root.join("r");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
    let result = store.delete_recording("r");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    if result.is_err() {
        assert!(dir.join("meta.txt").exists(), "メタが残っていること");
        let r = store.purge_expired(BASE + 10 * DAY, 1).unwrap();
        assert_eq!(r.deleted, ["r"], "次回は消せる");
    }
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_symlinked_recording_directory_is_never_followed_or_deleted() {
    let root = temp_root("purge-link");
    let target = root.join("elsewhere");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("keep.txt"), b"x").unwrap();
    std::os::unix::fs::symlink(&target, root.join("linked")).unwrap();
    let r = FsChunkStore::new(&root)
        .purge_expired(BASE + 1000 * DAY, 1)
        .unwrap();
    assert!(r.deleted.is_empty());
    assert!(target.join("keep.txt").exists());
    let _ = fs::remove_dir_all(&root);
}
