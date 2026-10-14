//! ファイルI/O関連の機能を提供するモジュール

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub fn load_local_file(path: &str) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("Failed to read file: {path}"))
}

/// Candidate bundle directories, most specific first.
///
/// Build artefacts (test indexes etc.) take priority over the CWD source tree,
/// so the `resource` directory next to the executable is tried first.
fn resource_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    // executable directory/resource
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        roots.push(dir.join("resource"));
    }

    // ./resource
    roots.push(PathBuf::from("resource"));

    // current_dir()/resource
    if let Ok(cd) = std::env::current_dir() {
        roots.push(cd.join("resource"));
    }

    roots
}

/// Loads `rel_path` relative to a single bundle `root`.
///
/// Returns `Ok(None)` when the file is absent under this root, and an error
/// when `rel_path` is not bundle-relative or resolves (through symlinks) outside
/// `root`.
pub(crate) fn load_resource_from(root: &Path, rel_path: &str) -> Result<Option<Vec<u8>>> {
    // A bundle-relative path may not be absolute nor climb out of the bundle
    // directory. Without this, `resource:///../../etc/passwd` would read any
    // file the process can.
    let rel = Path::new(rel_path);
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        anyhow::bail!("Refusing to load resource outside the bundle: {rel_path}");
    }

    let candidate = root.join(rel_path);
    if !candidate.is_file() {
        return Ok(None);
    }

    // Resolve symlinks before reading and confirm the final path still sits
    // under `root`, so a symlink cannot escape either.
    if let (Ok(resolved), Ok(root)) = (fs::canonicalize(&candidate), fs::canonicalize(root))
        && !resolved.starts_with(&root)
    {
        anyhow::bail!("Resource escapes the bundle directory: {rel_path}");
    }

    Ok(Some(fs::read(&candidate).with_context(|| {
        format!("Failed to read resource {:?}", candidate)
    })?))
}

/// リソースファイルを探して読み込む。
/// 順序は以下の通り：
/// - 実行ファイルのあるディレクトリ/resource/<rel_path> (build.rs が同期・生成したリソース)
/// - ./resource/<rel_path>
/// - カレントディレクトリ/resource/<rel_path>
///
/// ビルド成果物 (test インデックス等) が CWD のソースツリーより優先されるよう、
/// 実行ファイルの隣にある resource を最初に試す。
pub fn load_resource(rel_path: &str) -> Result<Vec<u8>> {
    let roots = resource_roots();
    for root in &roots {
        if let Some(bytes) = load_resource_from(root, rel_path)? {
            return Ok(bytes);
        }
    }

    // Surface the path validation error even when there are no roots to probe.
    if roots.is_empty() {
        load_resource_from(Path::new("resource"), rel_path)?;
    }

    anyhow::bail!("Resource not found: {}", rel_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn rejects_absolute_paths() {
        assert!(load_resource("/etc/passwd").is_err());
    }

    #[test]
    fn rejects_parent_directory_traversal() {
        let err = load_resource("../../../../etc/passwd").unwrap_err();
        assert!(
            err.to_string().contains("outside the bundle"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn loads_a_bundled_resource() {
        let bytes = load_resource("user-agent.css").expect("user-agent.css is bundled");
        assert!(!bytes.is_empty());
    }

    /// Creates an empty temporary directory that the caller must remove.
    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "orinium-io-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(name);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loads_nested_path_inside_root() {
        let root = temp_dir();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/file.txt"), b"nested").unwrap();

        let bytes = load_resource_from(&root, "sub/file.txt").unwrap();
        assert_eq!(bytes.as_deref(), Some(&b"nested"[..]));
        assert_eq!(load_resource_from(&root, "sub/missing.txt").unwrap(), None);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn from_root_rejects_absolute_and_parent_paths() {
        let root = temp_dir();
        assert!(load_resource_from(&root, "/etc/passwd").is_err());
        assert!(load_resource_from(&root, "../../etc/passwd").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escaping_root() {
        let root = temp_dir();
        let outside = temp_dir();
        fs::write(outside.join("secret.txt"), b"secret").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link.txt")).unwrap();

        let err = load_resource_from(&root, "link.txt").unwrap_err();
        assert!(
            err.to_string().contains("escapes the bundle"),
            "unexpected error: {err}"
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn allows_symlink_staying_inside_root() {
        let root = temp_dir();
        fs::write(root.join("real.txt"), b"inside").unwrap();
        std::os::unix::fs::symlink(root.join("real.txt"), root.join("alias.txt")).unwrap();

        let bytes = load_resource_from(&root, "alias.txt").unwrap();
        assert_eq!(bytes.as_deref(), Some(&b"inside"[..]));

        let _ = fs::remove_dir_all(&root);
    }
}
