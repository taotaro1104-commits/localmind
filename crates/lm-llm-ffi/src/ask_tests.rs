use super::*;
use lm_llm::{GenerateError, Generator, LlmMinutesExtractor};
use std::sync::Mutex;

/// 返事を順に返し、受け取ったプロンプトを覚える偽のエンジン。
struct Script {
    replies: Mutex<Vec<Result<String, GenerateError>>>,
    prompts: Mutex<Vec<String>>,
}

impl Generator for &'static Script {
    fn generate(&self, p: &str, _g: &str, _n: usize) -> Result<String, GenerateError> {
        self.prompts.lock().unwrap().push(p.to_string());
        self.replies.lock().unwrap().remove(0)
    }
    fn cancel(&self) {}
    fn id(&self) -> String {
        "script".into()
    }
}

fn engine(replies: Vec<Result<String, GenerateError>>) -> (MinutesEngine, &'static Script) {
    let script: &'static Script = Box::leak(Box::new(Script {
        replies: Mutex::new(replies),
        prompts: Mutex::new(vec![]),
    }));
    (
        MinutesEngine::from_extractor(LlmMinutesExtractor::new(Box::new(script))),
        script,
    )
}

fn source(n: u32) -> WebSource {
    WebSource {
        title: format!("題名{n}"),
        url: format!("https://ja.wikipedia.org/wiki/{n}"),
        text: format!("本文{n}"),
    }
}

#[test]
fn answering_without_sources_needs_no_search() {
    let (e, script) = engine(vec![Ok("植物が光で糖を作るはたらきです。".into())]);
    let r = e.answer("光合成とは？".into(), vec![]).unwrap();
    assert_eq!(r.text, "植物が光で糖を作るはたらきです。");
    assert!(r.cited.is_empty() && r.sources.is_empty());
    assert_eq!(r.model_id, "script");
    assert_eq!(script.prompts.lock().unwrap().len(), 1);
}

#[test]
fn answering_with_sources_returns_only_real_citations_and_the_cleaned_sources() {
    let (e, _) = engine(vec![Ok("晴れです[1]。嘘の出典[9]。".into())]);
    let mut bad = source(2);
    bad.url = "https://insecure.example/".into(); // Wikipedia でない出典は使わない
    let r = e.answer("天気は？".into(), vec![source(1), bad]).unwrap();
    assert_eq!(r.cited, vec![1]);
    assert_eq!(r.text, "晴れです[1]。嘘の出典。");
    assert_eq!(r.sources, vec![source(1)]);
}

#[test]
fn plan_search_returns_the_query_to_show_the_user() {
    let (e, _) = engine(vec![Ok(
        r#"{"need_search":true,"query":"東京 天気"}"#.into()
    )]);
    let p = e.plan_search("東京の天気は？".into()).unwrap();
    assert_eq!(
        p,
        SearchPlan {
            need_search: true,
            query: "東京 天気".into()
        }
    );
}

#[test]
fn errors_map_to_ffi_errors_without_echoing_input() {
    let (e, script) = engine(vec![]);
    assert!(matches!(
        e.answer("  ".into(), vec![]),
        Err(LlmError::InvalidInput { .. })
    ));
    let long = "あ".repeat(max_question_chars() as usize + 1);
    assert!(matches!(
        e.plan_search(long),
        Err(LlmError::InvalidInput { .. })
    ));
    assert!(
        script.prompts.lock().unwrap().is_empty(),
        "不正な質問はモデルに届かない"
    );

    for (reply, check) in [
        (
            Err(GenerateError::Cancelled),
            (|e: &LlmError| matches!(e, LlmError::Cancelled)) as fn(&LlmError) -> bool,
        ),
        (Err(GenerateError::TooLong), |e| {
            matches!(e, LlmError::InvalidInput { .. })
        }),
        (
            Err(GenerateError::Failed("メモリ不足".into())),
            |e| matches!(e, LlmError::Engine { msg } if msg.contains("メモリ不足")),
        ),
        (Ok("   ".into()), |e| matches!(e, LlmError::Engine { .. })),
    ] {
        let (e, _) = engine(vec![reply]);
        let err = e.answer("質問".into(), vec![]).unwrap_err();
        assert!(check(&err), "{err:?}");
    }
}

#[test]
fn wikipedia_responses_are_parsed_and_foreign_urls_dropped() {
    let json = r#"{"query":{"pages":[
      {"title":"東京","index":1,"extract":"日本の首都。","fullurl":"https://ja.wikipedia.org/wiki/%E6%9D%B1%E4%BA%AC"},
      {"title":"偽","index":2,"extract":"本文","fullurl":"https://evil.example/"}]}}"#;
    let s = parse_wikipedia_sources(json.into());
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].title, "東京");
    assert!(parse_wikipedia_sources("壊れた".into()).is_empty());
}
