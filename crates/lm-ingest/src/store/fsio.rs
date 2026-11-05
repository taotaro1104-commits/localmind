//! ファイル入出力の下回り（原子的な書き込み、上限つきの読み込み、ディレクトリの確認）。
//!
//! - 書き込みは「新規作成の一時ファイル → fsync → rename → 親ディレクトリの fsync」。
//!   一時ファイルは `create_new` で作るので、事前に置かれたシンボリックリンクや同名ファイルを辿らない。
//! - 読み込みは開いたハンドルに対して通常ファイルかを確認し、上限バイト数で打ち切る
//!   （検査と読み込みの間にファイルが差し替えられても、巨大な確保をしない）。
//! - Unix ではファイルを所有者のみ読み書き可（0600）、ディレクトリを 0700 で作る。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::IngestError;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
/// 一時ファイル名が衝突したときの再試行回数。
const TMP_ATTEMPTS: usize = 16;

/// 呼び出しごとに一意な一時ファイルのパス（`<元の名前>.<pid>-<連番>.tmp`）。
pub(super) fn tmp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!("{name}.{}-{seq}.tmp", std::process::id()))
}

/// 新規作成だけを許す（既存のファイルやシンボリックリンクがあれば `AlreadyExists`）。
pub(super) fn create_new(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// 一意な名前の一時ファイルを新規作成する。名前が衝突したら連番を進めて再試行する。
fn create_tmp(path: &Path) -> std::io::Result<(fs::File, PathBuf)> {
    let mut last = None;
    for _ in 0..TMP_ATTEMPTS {
        let tmp = tmp_path(path);
        match create_new(&tmp) {
            Ok(file) => return Ok((file, tmp)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("一時ファイルを作れません")))
}

/// 親ディレクトリを fsync して、rename を電源断から守る（Unix のみ。他の OS では何もしない）。
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        if let Ok(handle) = fs::File::open(dir) {
            let _ = handle.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// 一時ファイルに書いて fsync し、目的のパスへ rename する。失敗したら一時ファイルを消す。
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), IngestError> {
    write_atomic_with(path, bytes, || {})
}

/// `write_atomic` の本体。`before_rename` は、新しい内容を一時ファイルへ書き終えて fsync した後・rename の直前に
/// 呼ばれる（この時点では目的のパスは旧い内容のまま、というテストの観測点）。
pub(super) fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    before_rename: impl FnOnce(),
) -> Result<(), IngestError> {
    let (mut file, tmp) = create_tmp(path)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        before_rename();
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    sync_parent(path);
    Ok(())
}

/// 目的のパスが無いときだけ、内容を原子的に作る（一時ファイルに書いて fsync し、ハードリンクで置く）。
/// 作ったら `true`、既にあれば `false`（既存の内容には触れない）。同時に呼ばれても成功するのは 1 つだけ。
/// 読み手には、中身が書き切られたファイルしか見えない。
pub(super) fn write_new_atomic(path: &Path, bytes: &[u8]) -> Result<bool, IngestError> {
    let (mut file, tmp) = create_tmp(path)?;
    let written = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    })();
    drop(file);
    let linked = written.and_then(|()| fs::hard_link(&tmp, path));
    let _ = fs::remove_file(&tmp);
    match linked {
        Ok(()) => {
            sync_parent(path);
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// 通常のファイルだけを、`max_bytes` までの大きさで読む。シンボリックリンクや特殊ファイル（FIFO など）、
/// 上限を超えるファイルは `Corrupt`。開く前にリンクを調べ、開いたハンドルでも種類とサイズを確認し、
/// 読み込みも上限で打ち切る（調べてから開くまでの差し替えがあっても、確保量は上限で抑える）。
pub(super) fn read_regular_limited(path: &Path, max_bytes: u64) -> Result<Vec<u8>, IngestError> {
    let corrupt = |m: String| IngestError::Corrupt(m);
    // 開く前にリンクそのものを調べる（リンクや FIFO などは開かない。FIFO を開くと書き手を待って止まる）。
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(corrupt("通常のファイルではないため読みません".into()));
    }
    let file = open_regular(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(corrupt("通常のファイルではないため読みません".into()));
    }
    if meta.len() > max_bytes {
        return Err(corrupt(format!(
            "ファイルが大きすぎます: {} バイト",
            meta.len()
        )));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(corrupt(format!(
            "ファイルが大きすぎます: {max_bytes} バイト超"
        )));
    }
    Ok(bytes)
}

/// 読み取り用に開く。Unix ではリンクをたどらず（`O_NOFOLLOW`）、FIFO でも書き手を待たない
/// （`O_NONBLOCK`）。調べてから開くまでの間に差し替えられても止まらないようにする。
#[cfg(unix)]
fn open_regular(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_regular(path: &Path) -> std::io::Result<fs::File> {
    fs::File::open(path)
}

/// 録音ディレクトリが通常のディレクトリとして存在するか。無ければ false、
/// シンボリックリンクなど通常のディレクトリでないものはエラー（リンク先を読み書き・削除しない）。
pub(super) fn real_dir_exists(dir: &Path) -> Result<bool, IngestError> {
    match fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => Ok(true),
        Ok(_) => Err(IngestError::Io(format!(
            "録音ディレクトリが通常のディレクトリではありません（{}）",
            dir.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// 録音ディレクトリを作る（無ければ）。既存のものが通常のディレクトリでなければ拒否する。
pub(super) fn ensure_real_dir(dir: &Path) -> Result<(), IngestError> {
    if !real_dir_exists(dir)? {
        create_private_dir(dir)?;
    }
    Ok(())
}

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
