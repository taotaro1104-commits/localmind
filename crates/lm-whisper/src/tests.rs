use super::*;

#[test]
fn the_default_config_is_japanese_fast_with_a_bounded_thread_count() {
    let c = WhisperConfig::default();
    assert_eq!((c.language.as_str(), c.quality), ("ja", Quality::Fast));
    assert!((1..=MAX_DEFAULT_THREADS).contains(&c.threads));
    c.validate().unwrap();
}

#[test]
fn bad_configs_are_refused_before_any_model_is_touched() {
    let bad = |language: &str, threads: usize| WhisperConfig {
        language: language.into(),
        quality: Quality::Fast,
        threads,
    };
    for c in [
        bad("", 1),
        bad("J", 1),
        bad("japanese", 1),
        bad("JA", 1),
        bad("j1", 1),
        bad("ja", 0),
        bad("ja", 65),
    ] {
        assert!(
            matches!(c.validate(), Err(IngestError::InvalidFormat(_))),
            "{c:?}"
        );
    }
    // 存在しないモデルでも、先に設定の誤りで断る（モデルを読みに行かない）。
    let missing = std::env::temp_dir()
        .join("lm-whisper-no-such")
        .join("m.bin");
    let err = WhisperTranscriber::open(&missing, &"0".repeat(64), bad("ja", 0));
    assert!(matches!(err, Err(OpenError::InvalidConfig(_))));
}

#[test]
fn a_missing_or_tampered_model_is_an_integrity_error_and_is_never_loaded() {
    let missing = std::env::temp_dir()
        .join("lm-whisper-no-such")
        .join("m.bin");
    let err = WhisperTranscriber::open(&missing, &"0".repeat(64), WhisperConfig::default());
    assert!(matches!(err, Err(OpenError::Integrity(_))), "見つからない");

    let dir = std::env::temp_dir().join(format!("lm-whisper-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fake.bin");
    std::fs::write(&path, vec![7u8; 2 * 1024 * 1024]).unwrap();
    let err = WhisperTranscriber::open(&path, &"0".repeat(64), WhisperConfig::default())
        .err()
        .unwrap();
    assert!(matches!(err, OpenError::Integrity(_)), "{err:?}");
    assert!(err.to_string().contains("一致しません"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_malformed_hash_is_a_config_error_not_an_integrity_error() {
    let missing = std::env::temp_dir()
        .join("lm-whisper-no-such")
        .join("m.bin");
    let err = WhisperTranscriber::open(&missing, "xyz", WhisperConfig::default());
    assert!(matches!(err, Err(OpenError::InvalidConfig(_))), "{err:?}");
}

#[test]
fn open_errors_convert_to_ingest_errors_without_losing_the_meaning() {
    let from = |e: OpenError| IngestError::from(e);
    assert!(matches!(
        from(OpenError::Integrity("a".into())),
        IngestError::Transcribe(_)
    ));
    assert!(matches!(
        from(OpenError::Engine("b".into())),
        IngestError::Transcribe(_)
    ));
    let cfg = || IngestError::InvalidFormat("c".into());
    assert_eq!(from(OpenError::InvalidConfig(cfg())), cfg());
}
