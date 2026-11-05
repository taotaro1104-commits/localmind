use super::*;
use crate::search::MAX_NOTE_BYTES;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// テストごとの専用ディレクトリ。Drop で消す。
struct TempDb(std::path::PathBuf);

impl TempDb {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lm-ffi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> String {
        self.0.join("index.db").to_string_lossy().into_owned()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn profile(nano: bool, gguf: bool) -> DeviceProfile {
    DeviceProfile {
        ram_mb: 8192,
        nano_available: nano,
        pc_reachable: false,
        gguf_installed: gguf,
    }
}

#[test]
fn routing_maps_through_the_ffi_types() {
    assert_eq!(
        decide_route(Task::ShortText, profile(true, true)).unwrap(),
        Route::Nano
    );
    assert_eq!(
        decide_route(Task::ShortText, profile(false, true)).unwrap(),
        Route::Gguf
    );
    assert_eq!(
        decide_route(Task::Transcription, profile(false, false)).unwrap(),
        Route::Whisper
    );
    assert!(matches!(
        decide_route(Task::DocumentRag, profile(true, false)),
        Err(FfiError::Route { .. })
    ));
}

#[test]
fn lexical_engine_finds_and_refuses() {
    let db = TempDb::new("lex");
    let e = SearchEngine::open_lexical(db.path()).unwrap();
    assert!(e.is_empty().unwrap());
    e.upsert("a".into(), "契約書の解除は第12条に定める".into())
        .unwrap();
    e.upsert("b".into(), "議事録の保存期間は30日".into())
        .unwrap();
    assert_eq!(e.len().unwrap(), 2);
    let r = e.search("第12条".into(), 3).unwrap();
    assert!(r.found, "{r:?}");
    assert_eq!(r.hits[0].id, "a");
    let none = e.search("宇宙旅行の費用".into(), 3).unwrap();
    assert!(!none.found && none.hits.is_empty(), "{none:?}");
    assert!(e.remove("a".into()).unwrap());
    assert!(!e.remove("a".into()).unwrap());
}

#[test]
fn invalid_input_is_an_error_not_a_panic() {
    let db = TempDb::new("bad");
    let e = SearchEngine::open_lexical(db.path()).unwrap();
    assert!(matches!(
        e.upsert(String::new(), "x".into()),
        Err(FfiError::Search { .. })
    ));
    assert!(matches!(
        e.search("q".into(), 0),
        Err(FfiError::Search { .. })
    ));
    // 巨大な k は上限に丸められ、パニックも確保の暴走もしない。
    e.upsert("a".into(), "テキスト".into()).unwrap();
    assert!(e.search("テキスト".into(), u32::MAX).is_ok());
}

#[test]
fn oversized_documents_are_refused() {
    let db = TempDb::new("big");
    let e = SearchEngine::open_lexical(db.path()).unwrap();
    let big = "あ".repeat(MAX_NOTE_BYTES / 3 + 1);
    assert!(matches!(
        e.upsert("a".into(), big),
        Err(FfiError::Search { .. })
    ));
    assert!(e.is_empty().unwrap());
}

#[test]
fn only_absolute_paths_without_parent_components_are_accepted() {
    let db = TempDb::new("path");
    assert!(SearchEngine::open_lexical(String::new()).is_err());
    assert!(
        SearchEngine::open_lexical("index.db".into()).is_err(),
        "相対パス"
    );
    let dotdot = db.0.join("..").join("x.db").to_string_lossy().into_owned();
    assert!(SearchEngine::open_lexical(dotdot).is_err(), "..");
    assert!(SearchEngine::open_lexical(db.path()).is_ok());
}

/// 呼び出しを数える埋め込み。次元・失敗・返す長さを切り替えられる。
struct Probe {
    fail: bool,
    dim: u32,
    returned_len: usize,
    queries: AtomicUsize,
    dim_calls: AtomicUsize,
}

impl Probe {
    fn new(dim: u32) -> Arc<Self> {
        Arc::new(Self {
            fail: false,
            dim,
            returned_len: dim as usize,
            queries: AtomicUsize::new(0),
            dim_calls: AtomicUsize::new(0),
        })
    }
}

impl ForeignEmbedder for Probe {
    fn embed_document(&self, text: String) -> Result<Vec<f32>, FfiError> {
        if self.fail {
            return Err(FfiError::Callback { msg: "boom".into() });
        }
        // 文字に応じて向きを変える（常に同じベクトルでは検索の検証にならない）。
        let mut v = vec![0.0; self.returned_len];
        let slot = text.chars().count() % self.returned_len.max(1);
        if let Some(x) = v.get_mut(slot) {
            *x = 1.0;
        }
        Ok(v)
    }
    fn embed_query(&self, text: String) -> Result<Vec<f32>, FfiError> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        self.embed_document(text)
    }
    fn dim(&self) -> Result<u32, FfiError> {
        self.dim_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.dim)
    }
    fn id(&self) -> Result<String, FfiError> {
        Ok("probe-test".into())
    }
}

#[test]
fn foreign_embedder_is_used_for_documents_and_queries() {
    let db = TempDb::new("emb");
    let p = Probe::new(8);
    let e = SearchEngine::open(db.path(), p.clone(), e5_search_config()).unwrap();
    e.upsert("a".into(), "予算案を承認した".into()).unwrap();
    assert_eq!(e.len().unwrap(), 1);
    e.search("予算案".into(), 3).unwrap();
    assert!(
        p.queries.load(Ordering::SeqCst) >= 1,
        "クエリ用の埋め込みを使う"
    );
    assert_eq!(
        p.dim_calls.load(Ordering::SeqCst),
        1,
        "dim は open で 1 度だけ読む"
    );
}

#[test]
fn embedding_failures_keep_their_own_error_kind() {
    let db = TempDb::new("emb-ng");
    let ng = Arc::new(Probe {
        fail: true,
        ..Arc::try_unwrap(Probe::new(8)).ok().unwrap()
    });
    let e = SearchEngine::open(db.path(), ng, e5_search_config()).unwrap();
    match e.upsert("a".into(), "x".into()) {
        Err(FfiError::Embedding { msg }) => assert_eq!(msg, "boom", "二重に包まない"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_wrong_length_vector_from_the_callback_is_rejected() {
    let db = TempDb::new("emb-len");
    let bad = Arc::new(Probe {
        returned_len: 3, // dim は 8 と申告
        ..Arc::try_unwrap(Probe::new(8)).ok().unwrap()
    });
    let e = SearchEngine::open(db.path(), bad, e5_search_config()).unwrap();
    assert!(e.upsert("a".into(), "テキスト".into()).is_err());
    assert!(e.is_empty().unwrap());
}

#[test]
fn an_unusable_dim_is_rejected_at_open() {
    let db = TempDb::new("emb-dim");
    for dim in [0, 100_000] {
        assert!(
            SearchEngine::open(db.path(), Probe::new(dim), e5_search_config()).is_err(),
            "dim={dim}"
        );
    }
}

struct Throwing;

impl ForeignEmbedder for Throwing {
    fn embed_document(&self, _: String) -> Result<Vec<f32>, FfiError> {
        Ok(vec![1.0])
    }
    fn embed_query(&self, _: String) -> Result<Vec<f32>, FfiError> {
        Ok(vec![1.0])
    }
    fn dim(&self) -> Result<u32, FfiError> {
        Err(FfiError::Callback {
            msg: "no dim".into(),
        })
    }
    fn id(&self) -> Result<String, FfiError> {
        Ok("x".into())
    }
}

#[test]
fn a_throwing_dim_is_an_error_at_open_not_a_panic() {
    let db = TempDb::new("emb-throw");
    let r = SearchEngine::open(db.path(), Arc::new(Throwing), e5_search_config());
    assert!(matches!(r, Err(FfiError::Callback { .. })));
}

#[test]
fn invalid_config_is_rejected_at_open() {
    let db = TempDb::new("cfg");
    for (threshold, floor) in [(f32::NAN, 0.0), (-1.0, 0.0), (0.3, 1.0), (0.3, -0.1)] {
        let r = SearchEngine::open(
            db.path(),
            Probe::new(8),
            SearchConfig {
                grounding_threshold: threshold,
                semantic_floor: floor,
            },
        );
        assert!(
            matches!(r, Err(FfiError::Search { .. })),
            "threshold={threshold} floor={floor}"
        );
    }
}

#[cfg(feature = "onnx")]
mod e5 {
    use super::*;

    #[test]
    fn a_missing_or_tampered_model_dir_is_an_error() {
        let db = TempDb::new("e5-bad");
        let model = db.0.join("model");
        std::fs::create_dir_all(&model).unwrap();
        let tampered = db.0.join("tampered");
        std::fs::create_dir_all(&tampered).unwrap();
        for name in [
            "onnx__model.onnx",
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            std::fs::write(tampered.join(name), b"tampered").unwrap();
        }
        for dir in [model.clone(), tampered, db.0.join("nowhere")] {
            let r = SearchEngine::open_e5(
                db.path(),
                dir.to_string_lossy().into_owned(),
                e5_search_config(),
            );
            assert!(r.is_err(), "{dir:?}");
        }
        assert!(
            SearchEngine::open_e5("rel.db".into(), "m".into(), e5_search_config()).is_err(),
            "相対パス"
        );
        assert!(
            SearchEngine::open_e5(db.path(), "relative/model".into(), e5_search_config()).is_err(),
            "モデルの相対パス"
        );
    }

    #[test]
    fn only_an_absolute_libonnxruntime_file_is_accepted() {
        use crate::e5::is_expected_ort_library;
        let db = TempDb::new("ort-lib");
        let good = db.0.join("libonnxruntime.so");
        std::fs::write(&good, b"x").unwrap();
        assert!(is_expected_ort_library(&good));
        let other = db.0.join("libevil.so");
        std::fs::write(&other, b"x").unwrap();
        assert!(!is_expected_ort_library(&other), "別名");
        assert!(!is_expected_ort_library(
            &db.0.join("missing").join("libonnxruntime.so")
        ));
        let dotdot = db.0.join("sub").join("..").join("libonnxruntime.so");
        assert!(!is_expected_ort_library(&dotdot), "..");
        assert!(
            !is_expected_ort_library(std::path::Path::new("libonnxruntime.so")),
            "相対"
        );
        assert!(!is_expected_ort_library(&db.0), "ディレクトリ");
    }

    /// 実モデル。`LM_E5_DIR` に同梱形式のディレクトリ（`e5-small-<rev>` など）を指定して実行する。
    #[test]
    #[ignore = "実モデルが必要。LM_E5_DIR を指定して --ignored で実行する"]
    fn the_real_model_finds_and_refuses() {
        let dir = std::env::var("LM_E5_DIR").expect("LM_E5_DIR");
        let db = TempDb::new("e5-real");
        let e = SearchEngine::open_e5(db.path(), dir, e5_search_config()).unwrap();
        e.upsert("a".into(), "契約書の解除は第12条に定める".into())
            .unwrap();
        e.upsert("b".into(), "議事録の保存期間は30日".into())
            .unwrap();
        let hit = e.search("契約を解除するには".into(), 2).unwrap();
        assert!(hit.found && hit.hits[0].id == "a", "{hit:?}");
        let none = e.search("宇宙旅行の費用はいくら".into(), 2).unwrap();
        assert!(!none.found, "{none:?}");
    }
}

#[test]
fn search_errors_map_to_distinct_ffi_errors() {
    use lm_search::SearchError;
    assert!(matches!(
        FfiError::from(SearchError::Embedding("net".into())),
        FfiError::Embedding { .. }
    ));
    assert!(matches!(
        FfiError::from(SearchError::Integrity("hash".into())),
        FfiError::Integrity { .. }
    ));
    assert!(matches!(
        FfiError::from(SearchError::Config("c".into())),
        FfiError::Search { .. }
    ));
    assert!(matches!(
        FfiError::from(SearchError::Storage("s".into())),
        FfiError::Search { .. }
    ));
}
