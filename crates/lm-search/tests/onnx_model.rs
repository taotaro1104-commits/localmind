//! 実モデル（multilingual-e5-small）の結合テスト。モデルの取得とメモリが必要なので `#[ignore]`。
//!
//! 実行: LM_ONNX_CACHE=<モデルのキャッシュ先> \
//!       cargo test --release -p lm-search --features onnx --test onnx_model -- --ignored
#![cfg(feature = "onnx")]

use std::path::PathBuf;

use lm_search::eval::{build_searcher_custom, evaluate_searcher};
use lm_search::vector::cosine;
use lm_search::{Embedder, HybridConfig, NgramTokenizer, OnnxEmbedder};

fn cache_dir() -> PathBuf {
    std::env::var_os("LM_ONNX_CACHE")
        .map(PathBuf::from)
        .expect("環境変数 LM_ONNX_CACHE にモデルのキャッシュ先を指定してください")
}

fn model() -> OnnxEmbedder {
    OnnxEmbedder::multilingual_e5_small(cache_dir()).expect("モデルの読み込み")
}

#[test]
#[ignore = "モデルの取得が必要。LM_ONNX_CACHE を指定して --ignored で実行する"]
fn embeddings_have_the_declared_shape_and_are_deterministic() {
    let m = model();
    let a = m.embed_query("契約の更新日はいつですか").unwrap();
    assert_eq!(a.len(), m.dim());
    assert!(a.iter().all(|x| x.is_finite()));
    assert_eq!(
        a,
        m.embed_query("契約の更新日はいつですか").unwrap(),
        "同じ入力は同じ結果"
    );
    // クエリ用と文書用は接頭辞が違うので、同じ文でもベクトルが異なる。
    let doc = m.embed_document("契約の更新日はいつですか").unwrap();
    assert_ne!(a, doc);
    assert!(cosine(&a, &doc) > 0.9, "同じ文なので十分近い");
}

#[test]
#[ignore = "モデルの取得が必要。LM_ONNX_CACHE を指定して --ignored で実行する"]
fn related_text_is_closer_than_unrelated_text() {
    let m = model();
    let q = m.embed_query("契約の更新日はいつですか").unwrap();
    let related = m
        .embed_document("業務委託契約の更新日は来年三月三十一日です。")
        .unwrap();
    let unrelated = m
        .embed_document("トマトの苗は日当たりの良い場所に植え、支柱を立てます。")
        .unwrap();
    assert!(cosine(&q, &related) > cosine(&q, &unrelated) + 0.03);
}

#[test]
#[ignore = "モデルの取得が必要。LM_ONNX_CACHE を指定して --ignored で実行する"]
fn the_recommended_preset_meets_the_quality_bar_on_the_eval_set() {
    let config = HybridConfig::onnx_e5();
    let searcher = build_searcher_custom(Box::new(NgramTokenizer), Box::new(model()), config);
    let r = evaluate_searcher(&searcher, config.grounding_threshold, 3);
    assert!(r.recall_at_k >= 0.95, "{r:?}");
    assert_eq!(r.false_accept_rate, 0.0, "{r:?}");
    assert!(r.false_reject_rate <= 0.05, "{r:?}");
}

#[test]
#[ignore = "モデルの取得が必要。LM_ONNX_CACHE を指定して --ignored で実行する"]
fn very_long_and_odd_inputs_do_not_fail() {
    let m = model();
    let long = "契約の更新日について。".repeat(5000);
    assert_eq!(m.embed_document(&long).unwrap().len(), m.dim());
    for odd in ["", "。。。", "a", "\u{0}", "第99条", &"あ".repeat(3)] {
        assert_eq!(m.embed_query(odd).unwrap().len(), m.dim(), "{odd:?}");
    }
}

/// SQLite（sqlite-vec の 384 次元・cosine）でも、インメモリと同じ品質基準を満たす。
#[cfg(feature = "sqlite")]
#[test]
#[ignore = "モデルの取得が必要。LM_ONNX_CACHE を指定して --ignored で実行する"]
fn sqlite_with_the_real_model_meets_the_same_quality_bar() {
    use std::sync::Arc;

    use lm_search::eval::CORPUS;
    use lm_search::{HybridSearcher, SqliteStore};

    let store = SqliteStore::in_memory(Box::new(NgramTokenizer), Box::new(model())).unwrap();
    for (id, text) in CORPUS {
        store.upsert(id, text).unwrap();
    }
    let store = Arc::new(store);
    let config = HybridConfig::onnx_e5();
    let searcher = HybridSearcher::new(store.clone(), store, config);
    let r = evaluate_searcher(&searcher, config.grounding_threshold, 3);
    assert!(r.recall_at_k >= 0.95, "{r:?}");
    assert_eq!(r.false_accept_rate, 0.0, "{r:?}");
    assert!(r.false_reject_rate <= 0.05, "{r:?}");
}

/// 取得済みキャッシュのリビジョン別ディレクトリは、同梱モデルの展開先と同じ並び（`__` 区切り）。
fn bundled_dir() -> PathBuf {
    cache_dir().join("e5-small-614241f622f5")
}

#[test]
#[ignore = "モデルの取得が必要。LM_ONNX_CACHE を指定して --ignored で実行する"]
fn the_bundled_dir_gives_the_same_embeddings_as_the_cache() {
    let a = model().embed_query("契約の更新日").unwrap();
    let b = OnnxEmbedder::from_bundled_dir(bundled_dir())
        .expect("同梱ディレクトリの読み込み")
        .embed_query("契約の更新日")
        .unwrap();
    assert_eq!(a, b);
}

#[test]
fn a_missing_or_tampered_bundle_is_refused_without_network() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("lm-bundle-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(OnnxEmbedder::from_bundled_dir(&dir).is_err(), "空");
    // 名前だけ正しくて中身が違うファイルは、ハッシュ不一致で拒否される。
    for name in [
        "onnx__model.onnx",
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ] {
        std::fs::write(dir.join(name), b"tampered").unwrap();
    }
    assert!(OnnxEmbedder::from_bundled_dir(&dir).is_err(), "改ざん");
    assert!(OnnxEmbedder::from_bundled_dir(dir.join("nowhere")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
