use std::{
    env, fs, io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// テストページの正規エントリ (タイトル・説明付き)。
/// resource/test の走査で見つかり、file が重複しないものは自動で "Other" グループに追加される。
struct TestPageMeta {
    file: &'static str,
    group: &'static str,
    title: &'static str,
    description: &'static str,
}

const TEST_PAGE_CATALOG: &[TestPageMeta] = &[
    // ── Layout ──
    TestPageMeta {
        file: "basic_layout.html",
        group: "Layout",
        title: "Basic Layout",
        description: "文書フローの基本: ブロック積み重ね・ネスト・インライン要素・ページ構造",
    },
    TestPageMeta {
        file: "box_model.html",
        group: "Layout",
        title: "Box Model",
        description: "margin / padding / border / box-sizing / display / overflow",
    },
    TestPageMeta {
        file: "flex_grid.html",
        group: "Layout",
        title: "Flex & Grid",
        description: "flex の方向・折返し・justify/align・grow/shrink と grid レイアウト",
    },
    TestPageMeta {
        file: "inline_text.html",
        group: "Layout",
        title: "Inline Layout",
        description: "インライン要素のネストと margin の挙動",
    },
    // ── CSS ──
    TestPageMeta {
        file: "css_color.html",
        group: "CSS",
        title: "Color & Gradient",
        description: "名前付き色 / hex / rgb / hsl / inherit・currentColor / グラデーション",
    },
    TestPageMeta {
        file: "css_length.html",
        group: "CSS",
        title: "Length Units",
        description: "px / em / rem / % / vw / vh / calc()",
    },
    TestPageMeta {
        file: "css_text.html",
        group: "CSS",
        title: "Text & Font",
        description: "text-decoration / text-transform / 文字間隔 / font 各種プロパティ",
    },
    TestPageMeta {
        file: "size_constraint.html",
        group: "CSS",
        title: "Size Constraint",
        description: "min/max-width/height によるサイズ制約",
    },
    TestPageMeta {
        file: "border_radius.html",
        group: "CSS",
        title: "Border Radius",
        description: "角丸ボーダー: 一様 / 楕円 / 角ごと指定 / 複数値ショートハンド / パーセント・超過半径",
    },
    TestPageMeta {
        file: "text_align_wrap.html",
        group: "CSS",
        title: "Text Align & Wrap",
        description: "text-align / 折り返し (Latin・CJK・混在) / line-height",
    },
    TestPageMeta {
        file: "selector_test.html",
        group: "CSS",
        title: "CSS Selector",
        description: "タグ / クラス / id / 子孫 / 複数クラス / 複合セレクタ",
    },
    // ── HTML ──
    TestPageMeta {
        file: "compatibility_test.html",
        group: "HTML",
        title: "HTML Compatibility",
        description: "HTML Living Standard の広範な要素 (テキスト・リスト・テーブル・メディア・フォーム) の互換性確認",
    },
    TestPageMeta {
        file: "table_form.html",
        group: "HTML",
        title: "Table & Form",
        description: "テーブルレイアウト (thead/tbody/tfoot) とフォーム (button)",
    },
    // ── JavaScript ──
    TestPageMeta {
        file: "js_test.html",
        group: "JavaScript",
        title: "DOM & Click",
        description: "基本的なDOM操作とonclickによるクリックイベント",
    },
    TestPageMeta {
        file: "external_classic_script.html",
        group: "JavaScript",
        title: "External Classic Script",
        description: "inlineと外部classic scriptを混在させた文書順実行",
    },
    TestPageMeta {
        file: "js_events.html",
        group: "JavaScript",
        title: "Events & Scheduling",
        description: "DOMContentLoaded / addEventListener / async / defer",
    },
    TestPageMeta {
        file: "js_selectors.html",
        group: "JavaScript",
        title: "DOM Query Selectors",
        description: "document / Element の querySelector と querySelectorAll",
    },
    TestPageMeta {
        file: "js_dom_mutation.html",
        group: "JavaScript",
        title: "DOM Creation & Mutation",
        description: "createElement / createTextNode / appendChild / remove / parentNode / children / classList",
    },
    TestPageMeta {
        file: "js_timers.html",
        group: "JavaScript",
        title: "Timers",
        description: "setTimeout / clearTimeout / setInterval / clearInterval",
    },
    TestPageMeta {
        file: "js_microtasks.html",
        group: "JavaScript",
        title: "Microtasks",
        description: "queueMicrotaskのFIFO順序とscript / timer / event後のcheckpoint",
    },
    TestPageMeta {
        file: "js_promises.html",
        group: "JavaScript",
        title: "Promises",
        description: "Promiseのresolve / reject / then / catch / chainingとmicrotask順序",
    },
    TestPageMeta {
        file: "js_arrow_functions.html",
        group: "JavaScript",
        title: "Arrow Functions",
        description: "arrow functionの引数・expression/block body・closure・lexical this・Promise callback",
    },
    TestPageMeta {
        file: "js_fetch.html",
        group: "JavaScript",
        title: "Fetch",
        description: "fetchによる取得、Responseの状態・URL・本文、ネットワークエラーのreject",
    },
    TestPageMeta {
        file: "js_headers.html",
        group: "JavaScript",
        title: "Headers",
        description: "Headersの取得・変更・コピー、大文字小文字の正規化、fetchとResponse.headers",
    },
    TestPageMeta {
        file: "js_request.html",
        group: "JavaScript",
        title: "Request",
        description: "RequestのURL・method・headers・コピー・options上書きとfetchへの受け渡し",
    },
];

/// テスト一覧に表示するグループ順。未登録ファイルは "Other" に載せ、
/// JavaScriptの手動テストは一覧の末尾にまとめる。
const TEST_GROUP_ORDER: &[&str] = &["Layout", "CSS", "HTML", "Other", "JavaScript"];

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

    generate_test_index(&dest_root);
}

/// テストインデックス (test.html) を生成する。
///
/// `resource/test/test.html` のテンプレート (`.test-list` の空 div) に、
/// カタログ + `resource/test/` の走査で見つかった一覧を埋め込み、
/// 同期済みリソース (`target/{profile}/resource/test/test.html`) に書き出す。
/// ランタイムは埋め込み済みのファイルをそのまま配信するだけなので、
/// DOM 操作などの特別な処理は不要。
fn generate_test_index(dest_root: &Path) {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let test_dir = Path::new(&manifest_dir).join("resource").join("test");
    if !test_dir.is_dir() {
        build_log(format_args!(
            "[BUILD] resource/test directory not found, skipping test index generation"
        ));
        return;
    }
    // 新規ファイルの追加/削除でも再生成されるようにディレクトリごと監視する
    println!("cargo:rerun-if-changed={}", test_dir.display());

    let mut discovered: Vec<String> = fs::read_dir(&test_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|f| f.ends_with(".html") && f != "test.html")
                .collect()
        })
        .unwrap_or_default();
    discovered.sort();

    let catalogued: Vec<&str> = TEST_PAGE_CATALOG.iter().map(|c| c.file).collect();
    let extras: Vec<&String> = discovered
        .iter()
        .filter(|f| !catalogued.contains(&f.as_str()))
        .collect();

    // 一覧 HTML を組み立てる
    let mut list_html = String::new();
    for group in TEST_GROUP_ORDER {
        let members: Vec<&TestPageMeta> = TEST_PAGE_CATALOG
            .iter()
            .filter(|m| m.group == *group)
            .collect();
        if members.is_empty() && *group != "Other" {
            continue;
        }
        if group == &"Other" && extras.is_empty() {
            continue;
        }

        list_html.push_str(&format!("            <h2>{}</h2>\n", escape_html(group)));
        list_html.push_str("            <ul>\n");
        for meta in &members {
            list_html.push_str(&format!(
                "                <li><a href=\"{}\">{}</a> — {}</li>\n",
                escape_html(meta.file),
                escape_html(meta.title),
                escape_html(meta.description),
            ));
        }
        if *group == "Other" {
            for file in &extras {
                list_html.push_str(&format!(
                    "                <li><a href=\"{}\">{}</a></li>\n",
                    escape_html(file),
                    escape_html(file),
                ));
            }
        }
        list_html.push_str("            </ul>\n");
    }

    // テンプレートを読み、プレースホルダを埋め込む
    let template_path = test_dir.join("test.html");
    let template = match fs::read_to_string(&template_path) {
        Ok(s) => s,
        Err(e) => {
            build_log(format_args!("[BUILD] failed reading template: {}", e));
            return;
        }
    };
    const PLACEHOLDER: &str = r#"<div class="test-list"></div>"#;
    if !template.contains(PLACEHOLDER) {
        println!(
            "cargo:warning=test.html template does not contain `{PLACEHOLDER}`; index not filled"
        );
        return;
    }
    let filled = template.replace(
        PLACEHOLDER,
        &format!(
            r#"<div class="test-list">
{list_html}            </div>"#
        ),
    );

    let dest = dest_root.join("test").join("test.html");
    if let Some(parent) = dest.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        build_log(format_args!("[BUILD] failed creating dir: {}", e));
        return;
    }
    match fs::write(&dest, &filled) {
        Ok(()) => build_log(format_args!(
            "[BUILD] generated test index: {} ({} entries)",
            dest.display(),
            TEST_PAGE_CATALOG.len() + extras.len()
        )),
        Err(e) => build_log(format_args!("[BUILD] failed writing test index: {}", e)),
    }
}

/// 生成した HTML にそのまま埋め込むための最小限のエスケープ。
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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

    if let Some(parent) = log_path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        println!("[BUILD] failed creating log dir: {}", e);
        return;
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
    if log_path.exists()
        && let Err(e) = fs::remove_file(&log_path)
    {
        println!("[BUILD] failed removing old build log: {}", e);
    }
}
