use anyhow::Context;

#[allow(dead_code)]
pub async fn load_local_file(path: &str) -> Result<Vec<u8>, anyhow::Error> {
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;
    let mut file = File::open(path).await.context("Failed to open file")?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .await
        .context("Failed to read file")?;
    Ok(contents)
}

/// リソースファイルを探して読み込む。
/// 順序は以下の通り：
/// - ./resource/<rel_path>
/// - 実行ファイルのあるディレクトリ/resource/<rel_path>
/// - カレントディレクトリ/resource/<rel_path>
pub async fn load_resource(rel_path: &str) -> Result<Vec<u8>, anyhow::Error> {
    use std::path::PathBuf;
    use tokio::fs;
    use tokio::io::AsyncReadExt;

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

    for cand in candidates.into_iter() {
        if cand.exists() && cand.is_file() {
            let mut f = fs::File::open(&cand).await.context(format!("Failed to open {:?}", cand))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).await.context("Failed to read file")?;
            return Ok(buf);
        }
    }

    Err(anyhow::anyhow!("Resource not found: {}", rel_path))
}
