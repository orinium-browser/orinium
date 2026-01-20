use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

#[allow(dead_code)]
pub fn load_local_file(path: &str) -> Result<Vec<u8>> {
    fs::read(path).with_context(|| format!("Failed to read file: {path}"))
}

/// リソースファイルを探して読み込む。
/// 順序は以下の通り：
/// - ./resource/<rel_path>
/// - 実行ファイルのあるディレクトリ/resource/<rel_path>
/// - カレントディレクトリ/resource/<rel_path>
pub fn load_resource(rel_path: &str) -> Result<Vec<u8>> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // ./resource/<rel_path>
    candidates.push(PathBuf::from("resource").join(rel_path));

    // executable directory/resource/<rel_path>
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("resource").join(rel_path));
        }
    }

    // current_dir()/resource/<rel_path>
    if let Ok(cd) = std::env::current_dir() {
        candidates.push(cd.join("resource").join(rel_path));
    }

    for cand in candidates {
        if cand.is_file() {
            return fs::read(&cand).with_context(|| format!("Failed to read resource {:?}", cand));
        }
    }

    anyhow::bail!("Resource not found: {}", rel_path)
}
