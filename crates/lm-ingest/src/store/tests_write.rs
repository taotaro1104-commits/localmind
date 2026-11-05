//! 書き込みの原子性と同時実行のテスト。

use super::tests::{chunk, save, temp_root};
use super::*;

// ---- 書き込みの原子性と同時実行 --------------------------------------------------------

/// 大きなチャンクを上書きし続けながら別スレッドで読み続けても、壊れた（途中の）内容は決して見えない。
/// 一時ファイルを経由しない非原子的な書き込みなら、読み手が切れたファイルを見る。
#[test]
fn readers_never_see_a_partially_written_chunk() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let root = temp_root("atomic");
    let store = Arc::new(FsChunkStore::new(&root));
    let mut big = chunk(0, 120_000);
    big.sample_rate = 16_000;
    save(&store, "r", &big).unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let (store, done) = (Arc::clone(&store), Arc::clone(&done));
        std::thread::spawn(move || {
            let mut corrupt_seen = 0;
            let mut ok_reads = 0;
            while !done.load(Ordering::Relaxed) {
                let r = store.load_chunks("r").unwrap();
                // 読み取りが書き込みのリネームと重なって Io エラーになる場合は skipped に入るだけで、破損とは別。
                if r.skipped.iter().any(|s| s.contains("壊れています")) {
                    corrupt_seen += 1;
                }
                ok_reads += r.chunks.len();
            }
            (corrupt_seen, ok_reads)
        })
    };
    for round in 0..40 {
        let mut next = big.clone();
        next.samples[0] = round as i16;
        save(&store, "r", &next).unwrap();
    }
    done.store(true, Ordering::Relaxed);
    let (corrupt_seen, ok_reads) = reader.join().unwrap();
    assert_eq!(corrupt_seen, 0, "途中の内容が見えた");
    assert!(ok_reads > 0, "読み手が 1 度も成功しなかった");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn temporary_file_names_are_unique_per_call() {
    let p = Path::new("dir").join("00000001.pcm");
    let a = tmp_path(&p);
    let b = tmp_path(&p);
    assert_ne!(a, b);
    for t in [&a, &b] {
        let name = t.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("00000001.pcm.") && name.ends_with(".tmp"),
            "{name}"
        );
    }
    // メタとチャンクの一時ファイルは名前が衝突しない。
    let meta = tmp_path(&Path::new("dir").join(META_FILE));
    assert_ne!(meta.file_name(), a.file_name());
}

/// 同じチャンクへの同時書き込みで、一時ファイルが衝突したり、壊れたチャンクが残ったりしない。
/// （OS によっては rename が競合して一部が失敗しうるので、成否ではなく結果の健全性を見る。）
#[test]
fn concurrent_writes_to_one_chunk_leave_a_valid_file_and_no_temp_files() {
    use std::sync::Arc;

    let root = temp_root("concurrent");
    let store = Arc::new(FsChunkStore::new(&root));
    save(&store, "r", &chunk(0, 200)).unwrap();
    let handles: Vec<_> = (0..6)
        .map(|t| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                for round in 0..30 {
                    let mut c = chunk(0, 200);
                    c.samples[0] = (t * 100 + round) as i16;
                    let _ = save(&store, "r", &c);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let r = store.load_chunks("r").unwrap();
    assert_eq!(r.chunks.len(), 1, "{:?}", r.skipped);
    assert!(
        r.skipped.is_empty(),
        "一時ファイルが残った: {:?}",
        r.skipped
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_failed_write_leaves_no_temporary_file() {
    let root = temp_root("failed-write");
    fs::create_dir_all(&root).unwrap();
    // 書き込み先が空でないディレクトリだと rename が失敗する。
    let target = root.join("00000000.pcm");
    fs::create_dir_all(target.join("inner")).unwrap();
    assert!(write_atomic(&target, b"data").is_err());
    let leftovers: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    let _ = fs::remove_dir_all(&root);
}

/// 目的のパスは rename の瞬間まで旧い内容のままで、新しい内容は一時ファイルに完成している。
/// （一時ファイルを経由しない非原子的な書き込みなら、この時点で目的のパスがすでに書き換わっている。）
#[test]
fn the_destination_keeps_its_old_content_until_the_rename() {
    let root = temp_root("staged");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("00000000.pcm");
    fs::write(&target, b"OLD CONTENT").unwrap();

    let mut observed = None;
    write_atomic_with(&target, b"NEW CONTENT", || {
        let temps: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        observed = Some((
            fs::read(&target).unwrap(),
            temps.len(),
            temps.first().map(|t| fs::read(t.path()).unwrap()),
        ));
    })
    .unwrap();

    let (dest_before, temp_count, temp_content) = observed.expect("フックが呼ばれていない");
    assert_eq!(
        dest_before, b"OLD CONTENT",
        "rename の前に目的のパスが書き換わった"
    );
    assert_eq!(temp_count, 1, "一時ファイルが 1 つあるはず");
    assert_eq!(temp_content.as_deref(), Some(&b"NEW CONTENT"[..]));
    assert_eq!(fs::read(&target).unwrap(), b"NEW CONTENT");
    assert!(
        fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")),
        "rename 後に一時ファイルが残った"
    );
    let _ = fs::remove_dir_all(&root);
}
