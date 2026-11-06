//! llama.cpp による生成エンジン（feature `llama`）。GGUF のモデルを読み、文法（GBNF）に合う出力だけを生成する。
//!
//! 温度 0（貪欲）で決定的に生成する。生成の中止は、トークンごと（プロンプトの読み込み中は 512 トークンごと）に
//! 確かめる。ネットワークには出ない。

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;

use crate::extractor::{GenerateError, Generator};
use crate::model::{verify_file, ModelError};

/// 1 回に処理するプロンプトのトークン数。
const N_BATCH: usize = 512;
/// 文脈長の既定（トークン）。
pub const DEFAULT_N_CTX: u32 = 8192;
/// 文脈長の範囲。
const N_CTX_RANGE: std::ops::RangeInclusive<u32> = 512..=32_768;
/// スレッド数の上限。
const MAX_THREADS: u32 = 8;
/// 1 回の生成にかけてよい秒数の既定と範囲（低速な端末で、終わらないまま電池を使い切らない）。
pub const DEFAULT_MAX_SECONDS: u64 = 900;
const MAX_SECONDS_RANGE: std::ops::RangeInclusive<u64> = 10..=3600;

/// 設定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LlamaConfig {
    pub threads: u32,
    pub n_ctx: u32,
    /// 1 回の生成の制限時間（秒）。超えたら止めて失敗にする。
    pub max_seconds: u64,
}

impl Default for LlamaConfig {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(2, usize::from) as u32;
        Self {
            threads: cores.clamp(1, 4),
            n_ctx: DEFAULT_N_CTX,
            max_seconds: DEFAULT_MAX_SECONDS,
        }
    }
}

/// エンジンを開けない理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// 設定・引数の誤り。
    InvalidConfig(String),
    /// モデルが無い・大きさが違う・内容が固定したハッシュと一致しない。入れ直せば直る。
    Integrity(String),
    /// モデルは正しいが、読み込めない（形式が違う・メモリが足りない）。
    Engine(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (Self::InvalidConfig(m) | Self::Integrity(m) | Self::Engine(m)) = self;
        f.write_str(m)
    }
}

/// llama.cpp の初期化は、プロセスで 1 度だけ。
fn backend() -> Result<&'static LlamaBackend, OpenError> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            let mut b =
                LlamaBackend::init().map_err(|_| "LLM エンジンを初期化できません".to_string())?;
            b.void_logs(); // llama.cpp のログは、モデルのパスなどを含むので出さない
            Ok(b)
        })
        .as_ref()
        .map_err(|m| OpenError::Engine(m.clone()))
}

pub struct LlamaGenerator {
    model: LlamaModel,
    config: LlamaConfig,
    id: String,
    cancel: Arc<AtomicBool>,
    /// 生成は 1 つずつ（文脈のメモリを抑える）。
    run: Mutex<()>,
}

impl LlamaGenerator {
    /// モデルを検証してから読み込む。読み込みに時間とメモリがかかる。UI スレッドから呼ばないこと。
    pub fn open(path: &Path, sha256: &str, config: LlamaConfig) -> Result<Self, OpenError> {
        if !(1..=MAX_THREADS).contains(&config.threads) {
            return Err(OpenError::InvalidConfig(format!(
                "スレッド数は 1〜{MAX_THREADS} でなければなりません"
            )));
        }
        if !N_CTX_RANGE.contains(&config.n_ctx) {
            return Err(OpenError::InvalidConfig(
                "文脈長が範囲外です（512〜32768）".into(),
            ));
        }
        if !MAX_SECONDS_RANGE.contains(&config.max_seconds) {
            return Err(OpenError::InvalidConfig(
                "制限時間が範囲外です（10〜3600 秒）".into(),
            ));
        }
        let stamp = verify_file(path, sha256).map_err(|e| match e {
            ModelError::InvalidArgument(m) => OpenError::InvalidConfig(m),
            ModelError::Integrity(m) => OpenError::Integrity(m),
        })?;
        let backend = backend()?;
        // 検証のあとに差し替えられていないか、読み込む直前に確かめる（llama.cpp はパスから開き直すため）。
        if !stamp.matches(path) {
            return Err(OpenError::Integrity(
                "モデルファイルが検証のあとに変更されました。置き直してください".into(),
            ));
        }
        let model = LlamaModel::load_from_file(backend, path, &LlamaModelParams::default())
            .map_err(|_| {
                OpenError::Engine(
                    "モデルを読み込めません（形式が違うか、メモリが足りません）".into(),
                )
            })?;
        Ok(Self {
            model,
            config,
            id: format!("llama.cpp:{}", &sha256[..12]),
            cancel: Arc::new(AtomicBool::new(false)),
            run: Mutex::new(()),
        })
    }

    /// 中止の要求と制限時間を確かめる。
    fn should_stop(&self, deadline: Instant) -> Result<(), GenerateError> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(GenerateError::Cancelled)
        } else if Instant::now() >= deadline {
            Err(failed(
                "時間がかかりすぎたので止めました。短い文字起こしでお試しください",
            ))
        } else {
            Ok(())
        }
    }
}

fn failed(what: &str) -> GenerateError {
    GenerateError::Failed(what.into())
}

impl Generator for LlamaGenerator {
    fn generate(
        &self,
        prompt: &str,
        grammar: &str,
        max_new_tokens: usize,
    ) -> Result<String, GenerateError> {
        let _guard = self.run.lock().unwrap_or_else(|e| e.into_inner());
        // 開始前に届いた中止は有効（ここでは消さない）。終わったら次の実行のために戻す。
        let result = self.generate_locked(prompt, grammar, max_new_tokens);
        self.cancel.store(false, Ordering::Relaxed);
        result
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn reset_cancel(&self) {
        self.cancel.store(false, Ordering::Relaxed);
    }

    fn id(&self) -> String {
        self.id.clone()
    }
}

impl LlamaGenerator {
    fn generate_locked(
        &self,
        prompt: &str,
        grammar: &str,
        max_new_tokens: usize,
    ) -> Result<String, GenerateError> {
        let deadline = Instant::now() + Duration::from_secs(self.config.max_seconds);
        let backend = backend().map_err(|e| failed(&e.to_string()))?;
        let vocab = self.model.vocab();
        let tokens = vocab.tokenize(prompt.as_bytes(), true, true);
        if tokens.is_empty() || tokens.len() + max_new_tokens > self.config.n_ctx as usize {
            return Err(GenerateError::TooLong);
        }
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(self.config.n_ctx))
            .with_n_batch(N_BATCH as u32)
            .with_n_threads(self.config.threads as i32)
            .with_n_threads_batch(self.config.threads as i32);
        let mut ctx = self
            .model
            .new_context(backend, params)
            .map_err(|_| failed("推論の準備に失敗しました（メモリが足りません）"))?;
        let mut sampler = if grammar.is_empty() {
            // 自由な文章。同じ語句の繰り返しだけ抑える。
            LlamaSampler::chain_simple([
                LlamaSampler::penalties(self.model.n_vocab(), 64, 1.1, 0.0, 0.0),
                LlamaSampler::greedy(),
            ])
        } else {
            LlamaSampler::chain_simple([
                LlamaSampler::grammar(&self.model, grammar, "root")
                    .map_err(|_| failed("出力の形の指定が不正です"))?,
                LlamaSampler::greedy(),
            ])
        };

        // プロンプトを読み込む（最後のトークンだけ、次のトークンの確率を求める）。
        let mut batch = LlamaBatch::new(N_BATCH, 1);
        let last = tokens.len() - 1;
        for (c, chunk) in tokens.chunks(N_BATCH).enumerate() {
            self.should_stop(deadline)?;
            batch.clear();
            for (j, token) in chunk.iter().enumerate() {
                let pos = c * N_BATCH + j;
                batch
                    .add(*token, pos as i32, &[0], pos == last)
                    .map_err(|_| failed("推論に失敗しました"))?;
            }
            ctx.decode(&mut batch)
                .map_err(|_| failed("推論に失敗しました"))?;
        }

        // 1 トークンずつ生成する（`sample` は選んだトークンを文法にも受け入れさせる）。
        let mut out: Vec<u8> = Vec::new();
        for step in 0..max_new_tokens {
            let pos = tokens.len() + step;
            self.should_stop(deadline)?;
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            if vocab.is_eog(token) {
                break;
            }
            out.extend(vocab.token_to_piece(token, false, None));
            batch.clear();
            batch
                .add(token, pos as i32, &[0], true)
                .map_err(|_| failed("推論に失敗しました"))?;
            ctx.decode(&mut batch)
                .map_err(|_| failed("推論に失敗しました"))?;
        }
        String::from_utf8(out).map_err(|_| failed("出力を文字として読めません"))
    }
}
