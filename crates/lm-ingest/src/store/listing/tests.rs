use super::*;
use crate::store::tests::{chunk, consented, save, temp_root};
use std::fs;

#[test]
fn a_missing_or_empty_root_lists_nothing() {
    let root = temp_root("list-empty");
    assert!(FsChunkStore::new(root.join("nowhere"))
        .list_recordings()
        .unwrap()
        .is_empty());
    fs::create_dir_all(&root).unwrap();
    assert!(FsChunkStore::new(&root)
        .list_recordings()
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn recordings_are_listed_with_meta_chunk_count_and_size_in_id_order() {
    let root = temp_root("list-basic");
    let store = FsChunkStore::new(&root);
    for id in ["b", "a"] {
        store.begin_recording(id, &consented()).unwrap();
    }
    for i in 0..3 {
        save(&store, "a", &chunk(i, 100)).unwrap();
    }
    save(&store, "b", &chunk(0, 10)).unwrap();
    let list = store.list_recordings().unwrap();
    assert_eq!(
        list.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(list[0].chunk_count, 3);
    assert_eq!(list[1].chunk_count, 1);
    assert_eq!(list[0].meta, Some(consented()));
    assert!(list[0].bytes > list[1].bytes && list[1].bytes > 0);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_recording_without_readable_meta_is_still_listed_so_it_can_be_deleted() {
    let root = temp_root("list-nometa");
    fs::create_dir_all(root.join("orphan")).unwrap();
    fs::write(root.join("orphan").join("meta.txt"), "壊れている").unwrap();
    let list = FsChunkStore::new(&root).list_recordings().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].meta, None);
    assert_eq!(list[0].chunk_count, 0);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn only_chunk_files_are_counted_and_unrelated_entries_are_ignored() {
    let root = temp_root("list-noise");
    let store = FsChunkStore::new(&root);
    store.begin_recording("r", &consented()).unwrap();
    save(&store, "r", &chunk(0, 10)).unwrap();
    fs::write(root.join("r").join("notes.txt"), b"x").unwrap();
    fs::write(root.join("r").join("00000001.pcm.tmp"), b"x").unwrap();
    fs::create_dir_all(root.join("r").join("00000002.pcm")).unwrap();
    fs::create_dir_all(root.join("Not Valid ID")).unwrap();
    fs::write(root.join("file.txt"), b"x").unwrap();
    let list = store.list_recordings().unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].chunk_count, 1);
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_symlinked_recording_directory_is_not_listed() {
    let root = temp_root("list-link");
    let target = root.join("elsewhere");
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&target, root.join("linked")).unwrap();
    let ids: Vec<String> = FsChunkStore::new(&root)
        .list_recordings()
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert!(!ids.contains(&"linked".to_string()), "{ids:?}");
    let _ = fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn a_symlinked_root_is_refused() {
    let root = temp_root("list-root-link");
    let real = root.join("real");
    fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, root.join("root")).unwrap();
    assert!(FsChunkStore::new(root.join("root"))
        .list_recordings()
        .is_err());
    let _ = fs::remove_dir_all(&root);
}
