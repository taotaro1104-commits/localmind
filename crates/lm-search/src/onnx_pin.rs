//! ONNX モデルの取得元の固定と完全性の検証。
//!
//! リビジョン（コミット）とファイルごとの SHA-256 を固定し、一致したバイト列だけを返す。
//! これにより、ミラーの偽装や、書き込める保存先でのファイル差し替えがあっても、
//! 差し替えられたモデルが ONNX Runtime やトークナイザのパーサに渡らない。
//!
//! - 取得は自前（`ureq`）: https のみ、リダイレクトは回数を制限、全体のタイムアウトあり。
//!   ダウンロード中も `max_bytes` で読み込みを打ち切る（申告サイズを信用しない）。
//! - `HF_*` の環境変数やトークンファイル、認証情報は参照・送信しない。取得先は固定の huggingface.co。
//!   プロキシの環境変数（`HTTPS_PROXY` など）は ureq の既定どおり尊重する（社内ネットワークなどで必要）。
//!   経由先に関わらず、ハッシュが一致しないものは使われない（妨害されても取得に失敗するだけ）。
//! - 保存先は引数の `cache_dir` のみ。検証に通ったバイト列だけを、一時ファイル経由で原子的に書き込む。
//!   不一致のキャッシュは、取得して検証したあとに同じパスへ置き換える（シンボリックリンクは辿らず、
//!   リンクそのものが置き換わる）。リンク先など保存先の外には触れない。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::index::SearchError;

const REPO_ID: &str = "intfloat/multilingual-e5-small";
/// 固定するリビジョン（コミット）。
pub(crate) const REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";
const HOST: &str = "https://huggingface.co";
/// リダイレクトの最大回数（CDN への転送のため 0 にはできない）。
const MAX_REDIRECTS: u32 = 5;
/// 取得（接続から読み切りまで）の下限の時間。ファイルの大きさに応じて延ばす。
const MIN_FETCH_SECONDS: u64 = 60;
/// 想定する最低の回線速度（バイト/秒）。上限サイズのファイルをこの速度で読める時間を許す。
const MIN_BYTES_PER_SECOND: u64 = 256 * 1024;

/// 固定するファイル（リポジトリ内のパス、SHA-256、最大バイト数）。
pub(crate) const MODEL_FILE: PinnedFile = PinnedFile {
    path: "onnx/model.onnx",
    sha256: "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
    max_bytes: 512 * 1024 * 1024,
};
pub(crate) const TOKENIZER_FILE: PinnedFile = PinnedFile {
    path: "tokenizer.json",
    sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    max_bytes: 64 * 1024 * 1024,
};
pub(crate) const CONFIG_FILE: PinnedFile = PinnedFile {
    path: "config.json",
    sha256: "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
    max_bytes: 1024 * 1024,
};
pub(crate) const SPECIAL_TOKENS_FILE: PinnedFile = PinnedFile {
    path: "special_tokens_map.json",
    sha256: "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
    max_bytes: 1024 * 1024,
};
pub(crate) const TOKENIZER_CONFIG_FILE: PinnedFile = PinnedFile {
    path: "tokenizer_config.json",
    sha256: "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
    max_bytes: 1024 * 1024,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PinnedFile {
    pub path: &'static str,
    pub sha256: &'static str,
    pub max_bytes: u64,
}

fn embedding_error(message: String) -> SearchError {
    SearchError::Embedding(message)
}

/// 保存先でのこのファイルのパス（リビジョンごとのディレクトリ。パスの区切りは `__` にする）。
fn cache_path(cache_dir: &Path, file: &PinnedFile) -> PathBuf {
    cache_dir
        .join(format!("e5-small-{}", &REVISION[..12]))
        .join(file.path.replace('/', "__"))
}

/// 同梱モデルの展開先ディレクトリでのパス（ファイル名はパス区切りを `__` にしたもの。ネットワークは使わない）。
pub(crate) fn bundled_path(dir: &Path, file: &PinnedFile) -> PathBuf {
    dir.join(file.path.replace('/', "__"))
}

/// `max_bytes` までを読み、SHA-256 が固定値と一致する場合だけ内容を返す。
pub(crate) fn read_verified(path: &Path, file: &PinnedFile) -> Result<Vec<u8>, SearchError> {
    // 開く前にリンクそのものを調べる（リンクや FIFO は開かない。FIFO を開くと書き手を待って止まる）。
    let meta = fs::symlink_metadata(path)
        .map_err(|e| embedding_error(format!("{} を調べられません: {e}", file.path)))?;
    if !meta.is_file() || meta.len() > file.max_bytes {
        return Err(embedding_error(format!(
            "{} が通常のファイルでない、または大きすぎます",
            file.path
        )));
    }
    let bytes = read_capped(fs::File::open(path), file, meta.len())?;
    verify_bytes(&bytes, file)?;
    Ok(bytes)
}

/// `max_bytes` を超えて読まない。超えたらエラー（申告サイズではなく、実際に読んだ量で判断する）。
///
/// `size_hint`（実際のファイルの大きさや Content-Length）で最初に 1 回だけ確保する
/// （約 470MB のモデルで、倍々の再確保によるピークを避ける）。ヒントは上限で頭打ちにする。
fn read_capped<R: Read>(
    reader: std::io::Result<R>,
    file: &PinnedFile,
    size_hint: u64,
) -> Result<Vec<u8>, SearchError> {
    let reader = reader.map_err(|e| embedding_error(format!("{} を開けません: {e}", file.path)))?;
    let capacity = usize::try_from(size_hint.min(file.max_bytes)).unwrap_or(0);
    let mut bytes = Vec::with_capacity(capacity);
    reader
        .take(file.max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| embedding_error(format!("{} を読めません: {e}", file.path)))?;
    if bytes.len() as u64 > file.max_bytes {
        return Err(embedding_error(format!(
            "{} が上限（{} バイト）を超えています",
            file.path, file.max_bytes
        )));
    }
    Ok(bytes)
}

/// 内容の SHA-256 が固定値と一致するか調べる。
pub(crate) fn verify_bytes(bytes: &[u8], file: &PinnedFile) -> Result<(), SearchError> {
    if bytes.len() as u64 > file.max_bytes {
        return Err(embedding_error(format!(
            "{} が上限（{} バイト）を超えています",
            file.path, file.max_bytes
        )));
    }
    let actual = hex(&Sha256::digest(bytes));
    if actual == file.sha256 {
        Ok(())
    } else {
        Err(embedding_error(format!(
            "{} のハッシュが固定値と一致しません（取得元の差し替え、または破損）: {actual}",
            file.path
        )))
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// 固定リビジョンのファイルを取得する（https のみ。サイズの上限・タイムアウトあり）。
fn download(file: &PinnedFile) -> Result<Vec<u8>, SearchError> {
    download_from(
        &format!("{HOST}/{REPO_ID}/resolve/{REVISION}/{}", file.path),
        file,
    )
}

fn download_from(url: &str, file: &PinnedFile) -> Result<Vec<u8>, SearchError> {
    let timeout = Duration::from_secs(MIN_FETCH_SECONDS + file.max_bytes / MIN_BYTES_PER_SECOND);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(MAX_REDIRECTS)
        .timeout_global(Some(timeout))
        .timeout_connect(Some(Duration::from_secs(30)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .build(),
        )
        .build()
        .into();
    let response = agent
        .get(url)
        .call()
        .map_err(|e| embedding_error(format!("{} を取得できません: {e}", file.path)))?;
    let hint = response.body().content_length().unwrap_or(0);
    read_capped(Ok(response.into_body().into_reader()), file, hint)
}

/// 保存先のディレクトリを作る（Unix では所有者のみ 0700）。
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// 一時ファイルに書いてから置き換える（読み手に書きかけを見せない）。
fn write_cache(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    create_private_dir(dir)?;
    let tmp = dir.join(format!(
        ".tmp-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// 検証済みのバイト列を返す。保存先にあって検証に通ればそれを使い、無い・不一致なら取得し直して
/// （検証に通ったものだけ）保存先へ置く。保存に失敗しても、検証済みの内容は返す。
pub(crate) fn load(cache_dir: &Path, file: &PinnedFile) -> Result<Vec<u8>, SearchError> {
    load_with(cache_dir, file, download)
}

fn load_with(
    cache_dir: &Path,
    file: &PinnedFile,
    fetch: impl FnOnce(&PinnedFile) -> Result<Vec<u8>, SearchError>,
) -> Result<Vec<u8>, SearchError> {
    let path = cache_path(cache_dir, file);
    if let Ok(bytes) = read_verified(&path, file) {
        return Ok(bytes);
    }
    // 同時の初回取得で同じファイルを何度もダウンロードしないよう、取得はプロセス内で直列にする。
    // 先に取得した側が保存していれば、あとの側は保存先から読める。
    static FETCH_LOCK: Mutex<()> = Mutex::new(());
    let _guard = FETCH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Ok(bytes) = read_verified(&path, file) {
        return Ok(bytes);
    }
    let bytes = fetch(file)?;
    verify_bytes(&bytes, file)?;
    // 保存できなくても動作は続ける（次回また取得する）。気づけるよう警告は出す。
    if let Err(e) = write_cache(&path, &bytes) {
        eprintln!(
            "警告: モデルを保存先に書けませんでした（起動のたびに取得し直します）: {} ({e})",
            path.display()
        );
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
