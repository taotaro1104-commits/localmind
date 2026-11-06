use super::*;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

fn temp(tag: &str) -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lm-whisper-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn model(dir: &Path, len: usize) -> (std::path::PathBuf, String) {
    let bytes: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    let path = dir.join("ggml-test.bin");
    fs::write(&path, &bytes).unwrap();
    (path, hex(&bytes))
}

#[test]
fn the_verified_bytes_are_returned_and_match_the_pinned_hash() {
    let dir = temp("ok");
    let (path, sha) = model(&dir, 2 * 1024 * 1024);
    let bytes = load_verified(&path, &sha).unwrap();
    assert_eq!(bytes.len(), 2 * 1024 * 1024);
    assert_eq!(hex(&bytes), sha, "返した内容そのものが、検証した内容");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_changed_file_is_refused_without_leaking_the_path() {
    let dir = temp("changed");
    let (path, sha) = model(&dir, 2 * 1024 * 1024);
    let mut bytes = fs::read(&path).unwrap();
    bytes[1000] ^= 1; // 1 ビットだけ違う
    fs::write(&path, bytes).unwrap();
    let err = load_verified(&path, &sha).unwrap_err();
    assert!(matches!(err, ModelError::Integrity(_)), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("一致しません"), "{text}");
    assert!(
        !text.contains(dir.to_string_lossy().as_ref()),
        "パスを出さない: {text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_tiny_oversized_and_non_regular_files_are_integrity_errors() {
    let dir = temp("bad");
    let sha = hex(b"x");
    for result in [
        load_verified(&dir.join("none.bin"), &sha),
        load_verified(&dir, &sha), // ディレクトリ
    ] {
        assert!(
            matches!(result, Err(ModelError::Integrity(_))),
            "{result:?}"
        );
    }
    let tiny = dir.join("tiny.bin");
    fs::write(&tiny, b"x").unwrap();
    assert!(matches!(
        load_verified(&tiny, &hex(b"x")),
        Err(ModelError::Integrity(_))
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn malformed_expected_hashes_are_an_argument_error_not_an_integrity_error() {
    let dir = temp("hash");
    let (path, sha) = model(&dir, 2 * 1024 * 1024);
    for bad in [
        "",
        "abc",
        &sha.to_uppercase(),
        &format!("{sha}0"),
        &sha.replace('a', "g"),
    ] {
        assert!(
            matches!(
                load_verified(&path, bad),
                Err(ModelError::InvalidArgument(_))
            ),
            "{bad:?}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_symlink_is_followed_only_to_content_that_matches_the_hash() {
    let dir = temp("link");
    let (path, sha) = model(&dir, 2 * 1024 * 1024);
    let link = dir.join("link.bin");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(
        load_verified(&link, &sha).is_ok(),
        "内容が一致していれば、リンク越しでも同じ内容"
    );
    assert!(load_verified(&link, &hex(b"other")).is_err());
    let _ = fs::remove_dir_all(&dir);
}
