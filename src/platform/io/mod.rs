//! ファイルI/O関連の機能を提供するモジュール

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

#[allow(dead_code)]
pub fn load_local_file(path: &str) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("Failed to read file: {path}"))
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
    // A bundle-relative path may not be absolute nor climb out of the bundle
    // directory. Without this, `resource:///../../etc/passwd` would read any
    // file the process can.
    let rel = std::path::Path::new(rel_path);
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

    let mut candidates: Vec<(PathBuf, PathBuf)> = Vec::new();

    // executable directory/resource/<rel_path>
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let root = dir.join("resource");
        candidates.push((root.clone(), root.join(rel_path)));
    }

    // ./resource/<rel_path>
    let root = PathBuf::from("resource");
    candidates.push((root.clone(), root.join(rel_path)));

    // current_dir()/resource/<rel_path>
    if let Ok(cd) = std::env::current_dir() {
        let root = cd.join("resource");
        candidates.push((root.clone(), root.join(rel_path)));
    }

    for (root, cand) in candidates {
        if !cand.is_file() {
            continue;
        }
        // Resolve symlinks before reading and confirm the final path still sits
        // under the `resource` root, so a symlink cannot escape either.
        if let (Ok(resolved), Ok(root)) = (fs::canonicalize(&cand), fs::canonicalize(&root))
            && !resolved.starts_with(&root)
        {
            anyhow::bail!("Resource escapes the bundle directory: {rel_path}");
        }
        return fs::read(&cand).with_context(|| format!("Failed to read resource {:?}", cand));
    }

    anyhow::bail!("Resource not found: {}", rel_path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
