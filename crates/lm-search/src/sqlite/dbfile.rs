//! DB ファイルのパスと作成まわり（URI 解釈の回避、Unix での 0600）。

use std::path::Path;

use crate::index::SearchError;

/// DB のパスを絶対パスにする。`file:` で始まる相対パスが URI として解釈され、
/// `?vfs=` や `?mode=memory` で動作が変わるのを防ぐ。`:memory:` と空パスはそのまま。
pub(super) fn db_path(path: &Path) -> Result<std::path::PathBuf, SearchError> {
    if path.as_os_str().is_empty() || path.as_os_str() == ":memory:" {
        return Ok(path.to_path_buf());
    }
    std::path::absolute(path).map_err(|e| SearchError::Config(format!("DB のパスが不正です: {e}")))
}

/// DB ファイルを先に作る。Unix では所有者のみ読み書き可（0600）で作る（議事録の内容が推測できるため。
/// 作成してから chmod する方式だと、その間は他ユーザーに読めてしまう）。
/// 全 OS で、パスがシンボリックリンクや通常のファイル以外（ディレクトリ・FIFO など）なら拒否する。
/// 既存の通常ファイルには触れない。調べてから開くまでの差し替えは防げない（信頼する保存先が前提）。
pub(super) fn create_private_db_file(path: &Path) -> Result<(), SearchError> {
    let name = path.as_os_str();
    if name.is_empty() || name == ":memory:" {
        return Ok(());
    }
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => return Ok(()),
        Ok(_) => {
            return Err(SearchError::Config(
                "DB のパスが通常のファイルではありません（シンボリックリンクなど）".into(),
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(SearchError::Storage(e.to_string())),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(_) => Ok(()),
        // 直前に別のプロセスが作った場合は、そのまま開く。
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(SearchError::Storage(e.to_string())),
    }
}
