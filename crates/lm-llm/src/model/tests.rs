use super::*;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

fn temp(tag: &str) -> std::path::PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lm-llm-{tag}-{}-{}",
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

fn model(dir: &Path) -> (std::path::PathBuf, String) {
    let bytes: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let path = dir.join("m.gguf");
    fs::write(&path, &bytes).unwrap();
    (path, hex(&bytes))
}

#[test]
fn a_file_with_the_pinned_hash_passes() {
    let dir = temp("ok");
    let (path, sha) = model(&dir);
    verify_file(&path, &sha).unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_one_bit_change_is_refused_without_leaking_the_path() {
    let dir = temp("changed");
    let (path, sha) = model(&dir);
    let mut bytes = fs::read(&path).unwrap();
    bytes[5000] ^= 1;
    fs::write(&path, bytes).unwrap();
    let err = verify_file(&path, &sha).unwrap_err();
    assert!(matches!(err, ModelError::Integrity(_)));
    assert!(!err.to_string().contains(dir.to_string_lossy().as_ref()));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_tiny_and_non_regular_files_are_integrity_errors() {
    let dir = temp("bad");
    let sha = hex(b"x");
    assert!(matches!(
        verify_file(&dir.join("none"), &sha),
        Err(ModelError::Integrity(_))
    ));
    assert!(matches!(
        verify_file(&dir, &sha),
        Err(ModelError::Integrity(_))
    ));
    let tiny = dir.join("tiny");
    fs::write(&tiny, b"x").unwrap();
    assert!(matches!(
        verify_file(&tiny, &sha),
        Err(ModelError::Integrity(_))
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn malformed_hashes_are_an_argument_error() {
    let dir = temp("hash");
    let (path, sha) = model(&dir);
    for bad in ["", "abc", &sha.to_uppercase(), &format!("{sha}0")] {
        assert!(
            matches!(verify_file(&path, bad), Err(ModelError::InvalidArgument(_))),
            "{bad:?}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_file_changed_after_verification_no_longer_matches_its_stamp() {
    let dir = temp("stamp");
    let (path, sha) = model(&dir);
    let stamp = verify_file(&path, &sha).unwrap();
    assert!(stamp.matches(&path));
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(0);
    fs::write(&path, bytes).unwrap();
    assert!(
        !stamp.matches(&path),
        "差し替えを、読み込みの直前に見つける"
    );
    assert!(!stamp.matches(&dir.join("none")));
    let _ = fs::remove_dir_all(&dir);
}
