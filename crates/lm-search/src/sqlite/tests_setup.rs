//! 初期化のロック競合・WAL・書き込みロックのテスト。

use std::time::{Duration, Instant};

use super::setup::{enable_wal, setup_with_retry, SetupError};
use super::tests::{hashing, ngram, store};
use super::tests_persist::{cleanup, open, reopen_err, set_meta, temp_db};
use super::*;
use crate::tokenize::NgramTokenizer;
use crate::vector::HashingEmbedder;

// ---- 初期化のロック競合・WAL・書き込みロック -------------------------------------------

/// 別接続が書き込みロックを握り続けるとき、期限内に諦め、原因が分かるエラーを返す。
#[test]
fn setup_gives_up_within_the_deadline_with_a_clear_message() {
    let path = temp_db("deadline");
    drop(open(&path).unwrap());
    let holder = Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();

    // 期限を無視して待ち続けるバグでもテストがハングしないよう、別スレッドで実行して時間切れを失敗にする。
    let timeout = Duration::from_millis(300);
    let (tx, rx) = std::sync::mpsc::channel();
    let thread_path = path.clone();
    std::thread::spawn(move || {
        let mut conn = Connection::open(&thread_path).unwrap();
        let start = Instant::now();
        let r = setup_with_retry(
            &mut conn,
            &NgramTokenizer,
            &HashingEmbedder::new(64),
            false,
            timeout,
        );
        let _ = tx.send((r, start.elapsed()));
    });
    let (result, elapsed) = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("期限を過ぎても初期化が返ってこない（期限判定が壊れている）");
    let err = result.expect_err("ロック中は初期化できないはず");

    assert!(
        matches!(&err, SearchError::Storage(m) if m.contains("ロック競合")),
        "{err:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "期限 {timeout:?} に対し {elapsed:?} かかった"
    );
    drop(holder);
    cleanup(&path);
}

/// BUSY 以外の致命的なエラー（DB ファイルが壊れている等）はリトライせず、すぐ返す。
#[test]
fn fatal_errors_are_returned_immediately_without_retrying() {
    let path = temp_db("garbage");
    std::fs::write(&path, vec![0x5Au8; 8192]).unwrap();
    let mut conn = Connection::open(&path).unwrap();

    let timeout = Duration::from_secs(3);
    let start = Instant::now();
    let err = setup_with_retry(
        &mut conn,
        &NgramTokenizer,
        &HashingEmbedder::new(64),
        false,
        timeout,
    )
    .expect_err("壊れた DB は開けないはず");
    let elapsed = start.elapsed();

    assert!(matches!(err, SearchError::Storage(_)), "{err:?}");
    assert!(
        elapsed < Duration::from_secs(1),
        "致命的なエラーをリトライして {elapsed:?} 待った"
    );
    drop(conn);
    cleanup(&path);
}

#[test]
fn schema_is_not_trusted_after_open() {
    assert_eq!(store().trusted_schema(), 0);
    let path = temp_db("trust");
    let s = open(&path).unwrap();
    assert_eq!(s.trusted_schema(), 0);
    drop(s);
    cleanup(&path);
}

/// ロックが解放されたら、待機中の初期化が成功する。
/// （解放までの待ちは SQLite の busy_timeout 内で吸収される。BUSY を即時に返すリトライ分岐は
/// `concurrent_open_of_a_new_database_does_not_fail` が検証する。）
#[test]
fn setup_waits_for_a_released_lock_and_then_succeeds() {
    let path = temp_db("release");
    drop(open(&path).unwrap());
    let holder = Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        holder.execute_batch("COMMIT").unwrap();
    });

    let mut conn = Connection::open(&path).unwrap();
    let r = setup_with_retry(
        &mut conn,
        &NgramTokenizer,
        &HashingEmbedder::new(64),
        false,
        Duration::from_secs(3),
    );
    releaser.join().unwrap();
    assert!(r.is_ok(), "{r:?}");
    drop(conn);
    cleanup(&path);
}

#[test]
fn enable_wal_rejects_connections_that_cannot_use_wal() {
    // インメモリ DB の journal_mode は "memory" のままで WAL にできない。
    let conn = Connection::open_in_memory().unwrap();
    let r = enable_wal(&conn);
    assert!(matches!(
        r,
        Err(SetupError::Search(SearchError::Storage(_)))
    ));
}

/// 書き込みトランザクションは開始時に書き込みロックを取る（読み取りからの昇格で BUSY にならない）。
#[test]
fn begin_write_takes_the_write_lock_up_front() {
    let path = temp_db("immediate");
    let s = open(&path).unwrap();
    let mut guard = s.lock();
    let tx = begin_write(&mut guard).unwrap();

    let other = Connection::open(&path).unwrap();
    other.busy_timeout(Duration::ZERO).unwrap();
    assert!(
        other.execute_batch("BEGIN IMMEDIATE").is_err(),
        "begin_write が書き込みロックを取っていない（Deferred になっている）"
    );
    drop(tx);
    drop(guard);
    drop(other);
    drop(s);
    cleanup(&path);
}

#[test]
fn a_different_fts_schema_value_is_rejected() {
    let path = temp_db("ftsval");
    drop(open(&path).unwrap());
    set_meta(
        &path,
        "UPDATE meta SET value='old-schema' WHERE key='fts_schema'",
    );
    let err = reopen_err(&path, ngram(), hashing(64));
    assert!(
        matches!(&err, SearchError::Config(m) if m.contains("全文検索")),
        "{err:?}"
    );
    cleanup(&path);
}
