//! `onnx_pin` のテスト。

use super::*;

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ZERO_BYTE_SHA256: &str = "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d";

fn pin(sha256: &'static str, max_bytes: u64) -> PinnedFile {
    PinnedFile {
        path: "x/y",
        sha256,
        max_bytes,
    }
}

/// サーバのスレッドが接続待ちで残らないよう、最後に自分で接続して解放する。
struct TcpStreamGuard(u16);
impl Drop for TcpStreamGuard {
    fn drop(&mut self) {
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.0));
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lm-pin-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn matching_content_is_accepted_and_tampered_content_is_not() {
    assert!(verify_bytes(b"", &pin(EMPTY_SHA256, 10)).is_ok());
    let err = verify_bytes(b"x", &pin(EMPTY_SHA256, 10)).unwrap_err();
    assert!(err.to_string().contains("一致しません"), "{err}");
}

#[test]
fn integrity_failures_are_distinct_from_other_failures() {
    // ハッシュの不一致・サイズ超過・通常のファイルでないものは Integrity（展開し直せば直りうる）。
    assert!(matches!(
        verify_bytes(b"x", &pin(EMPTY_SHA256, 10)),
        Err(SearchError::Integrity(_))
    ));
    assert!(matches!(
        verify_bytes(b"\0", &pin(ZERO_BYTE_SHA256, 0)),
        Err(SearchError::Integrity(_))
    ));
    let dir = temp("integrity");
    assert!(matches!(
        read_verified(&dir, &pin(EMPTY_SHA256, 10)),
        Err(SearchError::Integrity(_))
    ));
    // ファイルが無いのも Integrity（展開し直せば直る）。ネットワーク失敗は Embedding のまま。
    assert!(matches!(
        read_verified(&dir.join("missing"), &pin(EMPTY_SHA256, 10)),
        Err(SearchError::Integrity(_))
    ));
}

#[test]
fn oversized_content_is_rejected_even_if_the_hash_matches() {
    assert!(verify_bytes(b"\0", &pin(ZERO_BYTE_SHA256, 1)).is_ok());
    assert!(verify_bytes(b"\0", &pin(ZERO_BYTE_SHA256, 0)).is_err());
}

#[test]
fn reading_stops_at_the_cap_whatever_the_source_claims() {
    // 申告サイズに関係なく、実際に読んだ量が上限を超えたらエラー（無限のストリームでも止まる）。
    let f = pin(EMPTY_SHA256, 100);
    assert!(read_capped(Ok(std::io::repeat(7)), &f, 0).is_err());
    assert_eq!(read_capped(Ok(&b"abc"[..]), &f, 3).unwrap(), b"abc");
    assert!(read_capped::<&[u8]>(Err(std::io::Error::other("x")), &f, 0).is_err());
}

#[test]
fn read_verified_rejects_a_modified_file_and_a_directory() {
    let dir = temp("verify");
    let f = dir.join("f");
    fs::write(&f, b"").unwrap();
    assert_eq!(read_verified(&f, &pin(EMPTY_SHA256, 10)).unwrap(), b"");
    fs::write(&f, b"tampered").unwrap();
    assert!(read_verified(&f, &pin(EMPTY_SHA256, 10)).is_err());
    assert!(read_verified(&dir, &pin(EMPTY_SHA256, 10)).is_err());
    assert!(read_verified(&dir.join("none"), &pin(EMPTY_SHA256, 10)).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_valid_cache_is_used_without_fetching() {
    let dir = temp("hit");
    let f = pin(EMPTY_SHA256, 10);
    write_cache(&cache_path(&dir, &f), b"").unwrap();
    let bytes = load_with(&dir, &f, |_| panic!("取得してはいけない")).unwrap();
    assert!(bytes.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_cache_is_fetched_verified_and_saved() {
    let dir = temp("miss");
    let f = pin(EMPTY_SHA256, 10);
    assert!(load_with(&dir, &f, |_| Ok(Vec::new())).unwrap().is_empty());
    assert!(cache_path(&dir, &f).is_file());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_tampered_cache_is_replaced_by_a_verified_download() {
    let dir = temp("tampered");
    let f = pin(EMPTY_SHA256, 10);
    let path = cache_path(&dir, &f);
    write_cache(&path, b"evil").unwrap();
    assert!(load_with(&dir, &f, |_| Ok(Vec::new())).unwrap().is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"", "検証済みの内容に置き換わる");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_download_that_fails_verification_is_neither_used_nor_saved() {
    let dir = temp("badfetch");
    let f = pin(EMPTY_SHA256, 10);
    let err = load_with(&dir, &f, |_| Ok(b"evil".to_vec())).unwrap_err();
    assert!(err.to_string().contains("一致しません"), "{err}");
    assert!(!cache_path(&dir, &f).exists());
    assert!(load_with(&dir, &f, |_| Err(embedding_error("net".into()))).is_err());
    let _ = fs::remove_dir_all(&dir);
}

/// 保存先の中のリンクが外のファイルを指していても、外のファイルは消えない・書き換わらない。
#[cfg(unix)]
#[test]
fn a_symlinked_cache_entry_never_touches_its_target() {
    let dir = temp("symlink");
    let f = pin(EMPTY_SHA256, 10);
    let outside = dir.join("outside.txt");
    fs::write(&outside, b"precious").unwrap();
    let path = cache_path(&dir, &f);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    assert!(load_with(&dir, &f, |_| Ok(Vec::new())).unwrap().is_empty());
    assert_eq!(fs::read(&outside).unwrap(), b"precious");
    let _ = fs::remove_dir_all(&dir);
}

/// https 以外（http）のサーバへは、接続を拒否する。
#[test]
fn plain_http_is_refused() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
    let _guard = TcpStreamGuard(port);
    let err = download_from(
        &format!("http://127.0.0.1:{port}/x"),
        &pin(EMPTY_SHA256, 10),
    )
    .unwrap_err();
    assert!(err.to_string().contains("取得できません"), "{err}");
    drop(_guard);
    let _ = server.join();
}

/// TLS として話せない相手（https の URL なのに平文を返す、すぐ切る）への接続は失敗し、
/// 取得失敗（Embedding）として返る。ネットワーク失敗は Integrity ではない。
#[test]
fn a_server_that_does_not_speak_tls_fails_as_a_download_error() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
    let _guard = TcpStreamGuard(port);
    let err = download_from(
        &format!("https://127.0.0.1:{port}/x"),
        &pin(EMPTY_SHA256, 10),
    )
    .unwrap_err();
    assert!(matches!(err, SearchError::Embedding(_)), "{err:?}");
    assert!(err.to_string().contains("取得できません"), "{err}");
    drop(_guard);
    let _ = server.join();
}

/// 同時の初回取得でも、ダウンロードは 1 回だけで、あとの側は保存先から読む。
#[test]
fn concurrent_first_loads_download_once() {
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    let dir = temp("once");
    let count = Arc::new(AtomicUsize::new(0));
    let f = pin(EMPTY_SHA256, 10);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let (dir, count) = (dir.clone(), Arc::clone(&count));
            std::thread::spawn(move || {
                load_with(&dir, &f, |_| {
                    count.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(50));
                    Ok(Vec::new())
                })
                .unwrap()
            })
        })
        .collect();
    for h in handles {
        assert!(h.join().unwrap().is_empty());
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pinned_hashes_are_well_formed() {
    for f in [
        MODEL_FILE,
        TOKENIZER_FILE,
        CONFIG_FILE,
        SPECIAL_TOKENS_FILE,
        TOKENIZER_CONFIG_FILE,
    ] {
        assert_eq!(f.sha256.len(), 64, "{}", f.path);
        assert!(
            f.sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "小文字の 16 進のみ: {}",
            f.path
        );
    }
    assert_eq!(REVISION.len(), 40);
}
