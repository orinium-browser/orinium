use std::{
    env, fs, io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    clear_build_log();

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());

    let target_dir = env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let mut p = std::path::PathBuf::from(&manifest_dir);
        p.push("target");
        p.to_string_lossy().into_owned()
    });

    let src_root = Path::new(&manifest_dir).join("resource");
    if !src_root.exists() {
        build_log(format_args!(
            "[BUILD] resource directory not found at {}",
            src_root.display()
        ));
        return;
    }

    let dest_root = Path::new(&target_dir).join(&profile).join("resource");

    if let Err(e) = visit_files(&src_root, &|p| {
        build_log(format_args!("cargo:rerun-if-changed={}", p.display()))
    }) {
        build_log(format_args!("[BUILD] failed reading resource tree: {}", e));
    }

    if let Err(e) = copy_dir_if_newer(&src_root, &src_root, &dest_root) {
        build_log(format_args!("[BUILD] failed copying resources: {}", e));
    } else {
        build_log(format_args!(
            "[BUILD] resource sync completed -> {}",
            dest_root.display()
        ));
    }
}

fn visit_files<F: Fn(&Path)>(dir: &Path, cb: &F) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            visit_files(&path, cb)?;
        } else if path.is_file() {
            cb(&path);
        }
    }
    Ok(())
}

/// コピー先が存在しないか、ソースの方が新しければコピーする
fn copy_dir_if_newer(root: &Path, current: &Path, dst_root: &Path) -> io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let src_path = entry.path();
        if src_path.is_dir() {
            copy_dir_if_newer(root, &src_path, dst_root)?;
            continue;
        }
        if src_path.is_file() {
            let rel = src_path.strip_prefix(root).unwrap();
            let dst_path = dst_root.join(rel);

            let need_copy = match dst_path.metadata() {
                Ok(dst_meta) => {
                    let src_meta = src_path.metadata()?;
                    match (src_meta.modified(), dst_meta.modified()) {
                        (Ok(sm), Ok(dm)) => sm > dm,
                        _ => true,
                    }
                }
                Err(_) => true,
            };

            if need_copy {
                if let Some(p) = dst_path.parent() {
                    fs::create_dir_all(p)?;
                }
                fs::copy(&src_path, &dst_path)?;
                build_log(format_args!(
                    "[BUILD] copied resource: {} -> {}",
                    src_path.display(),
                    dst_path.display()
                ));
            }
        }
    }
    Ok(())
}
/// target/{profile}/build.logにビルドログを書き込む
fn build_log(args: std::fmt::Arguments) {
    use std::io::Write;
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target_dir = env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let mut p = std::path::PathBuf::from(&manifest_dir);
        p.push("target");
        p.to_string_lossy().into_owned()
    });

    let log_path = Path::new(&target_dir).join(&profile).join("build.log");

    if let Some(parent) = log_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            println!("[BUILD] failed creating log dir: {}", e);
            return;
        }
    }

    let now = SystemTime::now();
    let unixtime = now.duration_since(UNIX_EPOCH).expect("back to the future");

    let msg = format!("{}", args);
    let content = format!("[{}] {}", unixtime.as_secs(), msg);

    match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(mut f) => {
            if let Err(e) = writeln!(f, "{}", content) {
                println!("[BUILD] failed writing build log: {}", e);
            }
        }
        Err(e) => println!("[BUILD] failed opening build log: {}", e),
    }
}

fn clear_build_log() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target_dir = env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let mut p = std::path::PathBuf::from(&manifest_dir);
        p.push("target");
        p.to_string_lossy().into_owned()
    });
    let log_path = Path::new(&target_dir).join(&profile).join("build.log");
    if log_path.exists() {
        if let Err(e) = fs::remove_file(&log_path) {
            println!("[BUILD] failed removing old build log: {}", e);
        }
    }
}
