use orinium_browser::{
    ProcessHandler,
    browser::{BrowserApp, BrowserUi, Tab, core::resource_loader::BrowserResourceLoader},
    engine::{
        css::matcher::ElementChain,
        css::parser::Parser as CssParser,
        html::{
            HtmlNodeType,
            parser::{ClassicScriptExecution, DomTree, Parser as HtmlParser},
        },
        js::{JsRuntime, missing_api_detector},
        layouter::{
            InheritedCss, build_layout_and_info,
            css_resolver::{CssResolver, ResolvedStyles, StyleOrigin, append_resolved_styles},
            types::{TextFlowStyle, TextStyle},
        },
        renderer_model::generate_draw_commands,
        tree::NodeRef,
    },
    platform::{
        network::{NetworkConfig, NetworkCore},
        renderer::text_measurer::PlatformTextMeasurer,
    },
};

use colored::*;

use anyhow::Result;
use std::{env, rc::Rc, sync::Arc};
use ui_layout::{LayoutEngine, LayoutNode};

fn main() -> Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    env_logger::init();

    if let Some(handler) = ProcessHandler::current() {
        handler.handle();
    }

    let args: Vec<String> = env::args().collect();
    if args.len() >= 2 {
        match args[1].as_str() {
            "help" => {
                if args.len() == 3 {
                    let command = &args[2];
                    let commands = get_commands();
                    if let Some((description, args, delail)) = commands.get(command.as_str()) {
                        println!(
                            "{}",
                            format!("Help for command: {}", command).bold().underline()
                        );
                        println!("\n{}:", "Description".bold());
                        println!("  {}", description);
                        println!("\n{}:", "Usage".bold());
                        println!("  cargo run --example tests {} {}", command, args);
                        if !delail.is_empty() {
                            println!("\n{}:", "Details".bold());
                            println!("  {}", delail);
                        }
                    } else {
                        eprintln!("Unknown command: {}", command);
                        let command_list: Vec<&str> = commands.keys().copied().collect();
                        if let Some(suggested) = suggest_command(command, &command_list) {
                            eprintln!("Did you mean: {} ?", suggested);
                        }
                    }
                } else {
                    let commands = get_commands();
                    println!("{}", "Orinium Browser Test Application".bold().underline());
                    println!("\n{}", "Usage:".bold());
                    println!("  cargo run --example tests [COMMAND] [ARGS]\n");

                    println!("{}", "Available Commands:".bold());
                    for (name, (description, args, _detail)) in &commands {
                        println!(
                            "  {:<15} {:<8} - {}",
                            name.green().bold(),
                            args.cyan(),
                            description
                        );
                    }

                    println!("\n{}", "Note:".bold());
                    println!("  - URLs must include the scheme (e.g. http://, https://, data:).");

                    println!("\nTo see more details about a specific command, run:");
                    println!("  cargo run --example tests help [COMMAND]");
                }
            }
            "parse_dom" => {
                if args.len() == 3 || args.len() == 4 || args.len() == 5 {
                    let url = &args[2];
                    println!("Parsing DOM for URL: {}", url);
                    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
                    let loader = BrowserResourceLoader::new(Some(Rc::new(net)));
                    let resp = loader
                        .fetch_blocking(url.parse()?)
                        .expect("Failed to fetch URL");
                    let html = String::from_utf8_lossy(&resp.body).to_string();
                    println!(
                        "Fetched HTML (first 50 chars):\n{}",
                        html.chars().take(50).collect::<String>()
                    );
                    let mut parser = HtmlParser::new(&html);
                    let dom = parser.parse();
                    if args.len() >= 4 {
                        let hide_tag_names: Vec<String> =
                            args[3].split(',').map(|s| s.to_ascii_lowercase()).collect();

                        let hidden_attr = if args.len() == 5 {
                            args[4].to_ascii_lowercase() == "true"
                        } else {
                            false
                        };

                        dom.traverse(&mut |node_rc: &NodeRef<HtmlNodeType>| {
                            let mut node = node_rc.borrow_mut();

                            if let Some(tag_name) = node.value.tag_name() {
                                if hide_tag_names
                                    .iter()
                                    .any(|hide: &String| hide == &tag_name.to_ascii_lowercase())
                                {
                                    node.clear_children();
                                }
                            }

                            if hidden_attr {
                                if let HtmlNodeType::Element { attributes, .. } = &mut node.value {
                                    attributes.clear();
                                }
                            }
                        });
                    }
                    println!("DOM Tree:\n{}", dom);
                } else {
                    eprintln!("Please provide a URL for DOM parsing test.");
                }
            }
            "parse_cssom" => {
                if args.len() == 3 {
                    let url = &args[2];
                    println!("Parsing CSSOM for URL: {}", url);
                    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
                    let loader = BrowserResourceLoader::new(Some(Rc::new(net)));
                    let resp = loader
                        .fetch_blocking(url.parse()?)
                        .expect("Failed to fetch URL");
                    let css = String::from_utf8_lossy(&resp.body).to_string();
                    println!(
                        "Fetched CSS (first 50 chars):\n{}",
                        css.chars().take(50).collect::<String>()
                    );
                    let mut parser = orinium_browser::engine::css::parser::Parser::new(&css);
                    let cssom = parser.parse()?;
                    println!("CSSOM Tree:\n{}", cssom);
                } else {
                    eprintln!("Please provide a URL for CSSOM parsing test.");
                }
            }
            "send_request" => {
                if args.len() == 3 {
                    let url = &args[2];
                    println!("Sending request to URL: {}", url);
                    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
                    net.set_network_config(NetworkConfig {
                        follow_redirects: false,
                        ..Default::default()
                    });
                    match net.fetch_blocking(url) {
                        Ok(resp) => {
                            println!("Response Status: {}", resp.status.as_u16());
                            println!("Response Headers:");
                            for (key, value) in &resp.headers {
                                println!("{}: {}", key, value);
                            }
                            println!("Response Body:");
                            println!("Body size: {} bytes", resp.body.len());

                            if let Ok(text) =
                                std::str::from_utf8(&resp.body[..resp.body.len().min(1024)])
                            {
                                println!("Preview:\n{}", text);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to send request: {}", e);
                        }
                    }
                } else {
                    eprintln!("Please provide a URL for sending request test.");
                }
            }
            "fetch_url" => {
                if args.len() == 3 {
                    let url = &args[2];
                    println!("Fetching URL: {}", url);
                    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
                    match net.fetch_blocking(url) {
                        Ok(resp) => {
                            println!("Response Reason-Phrase: {}", resp.reason_phrase);
                            println!("Response Headers:");
                            for (key, value) in &resp.headers {
                                println!("{}: {}", key, value);
                            }
                            println!("Response Body:");
                            println!("Body size: {} bytes", resp.body.len());

                            if let Ok(text) =
                                std::str::from_utf8(&resp.body[..resp.body.len().min(1024)])
                            {
                                println!("Preview:\n{}", text);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to fetch URL: {}", e);
                        }
                    }
                } else {
                    eprintln!("Please provide a URL for fetching test.");
                }
            }
            "dump_infonode" => {
                if args.len() == 3 {
                    let raw_url = &args[2];
                    println!("Dumping InfoNode for URL: {}", raw_url);

                    let info = build_layout_info(raw_url, (800.0, 600.0))?.info;

                    println!("\nInfoNode:\n{:#?}", info);
                } else {
                    eprintln!("Please provide a URL for dump_infonode test.");
                }
            }
            "dump_domnode" => {
                if args.len() == 3 {
                    let raw_url = &args[2];
                    println!("Dumping DOM node for URL: {}", raw_url);

                    let parsed_url: url::Url = raw_url.parse()?;
                    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
                    let loader = BrowserResourceLoader::new(Some(Rc::new(net)));
                    let resp = loader
                        .fetch_blocking(parsed_url)
                        .expect("Failed to fetch URL");
                    let html = String::from_utf8_lossy(&resp.body).to_string();

                    let mut parser = HtmlParser::new(&html);
                    let dom = Rc::new(parser.parse());

                    // Run inline scripts so DOM mutations are inspectable.
                    let mut js_runtime = JsRuntime::new(Rc::clone(&dom));
                    for script in dom.collect_inline_scripts() {
                        js_runtime.run_script(&script);
                    }

                    println!("DOM Tree:\n{}", dom);
                } else {
                    eprintln!("Please provide a URL for dump_domnode test.");
                }
            }
            "dump_layoutnode" => {
                if args.len() == 3 || args.len() == 5 {
                    let raw_url = &args[2];
                    let (viewport_w, viewport_h) = viewport_args(&args, 800.0, 600.0)?;
                    println!("Dumping LayoutNode for URL: {}", raw_url);

                    let mut ctx = build_layout_info(raw_url, (viewport_w, viewport_h))?;
                    LayoutEngine::layout(&mut ctx.layout, viewport_w, viewport_h);

                    println!("\nLayoutNode:\n{:#}", ctx.layout);
                } else {
                    eprintln!(
                        "Please provide a URL and optional viewport size (URL [WIDTH HEIGHT])."
                    );
                }
            }
            "dump_draw_command" => {
                if args.len() == 3 || args.len() == 5 {
                    let raw_url = &args[2];
                    let (viewport_w, viewport_h) = viewport_args(&args, 800.0, 600.0)?;
                    println!("Dumping draw commands for URL: {}", raw_url);

                    let mut ctx = build_layout_info(raw_url, (viewport_w, viewport_h))?;
                    LayoutEngine::layout(&mut ctx.layout, viewport_w, viewport_h);

                    let mut draw_commands = Vec::new();
                    generate_draw_commands(
                        &mut draw_commands,
                        &ctx.layout,
                        &ctx.info,
                        (viewport_w, viewport_h),
                    );

                    println!("\nGenerated {} draw commands:", draw_commands.len());
                    for (i, cmd) in draw_commands.iter().enumerate() {
                        println!("  [{:>3}] {:?}", i, cmd);
                    }
                } else {
                    eprintln!(
                        "Please provide a URL and optional viewport size (URL [WIDTH HEIGHT])."
                    );
                }
            }
            "run_layout" => {
                if args.len() == 3 || args.len() == 5 {
                    let raw_url = &args[2];
                    let (viewport_w, viewport_h) = viewport_args(&args, 800.0, 600.0)?;
                    println!("Running layout for URL: {}", raw_url);

                    let mut ctx = build_layout_info(raw_url, (viewport_w, viewport_h))?;
                    LayoutEngine::layout(&mut ctx.layout, viewport_w, viewport_h);

                    println!("Done!");
                } else {
                    eprintln!(
                        "Please provide a URL and optional viewport size (URL [WIDTH HEIGHT])."
                    );
                }
            }
            "simple_render" => {
                if args.len() == 3 {
                    let url = &args[2];
                    println!("Testing simple rendering for URL: {}", url);

                    let mut tab = Tab::default();
                    tab.navigate(url.parse()?);

                    let mut browser = BrowserApp::default();
                    browser.set_default_ui(BrowserUi::with_tab(tab));

                    browser.run()?
                } else {
                    eprintln!("Please provide a URL for simple rendering test.");
                }
            }
            "webcompat" => {
                if args.len() == 3 {
                    run_webcompat(&args[2])?;
                } else {
                    eprintln!("Please provide a URL for the webcompat report.");
                }
            }
            _ => {
                eprintln!("Unknown argument: {}", args[1]);
                let commands: Vec<&str> = get_commands().keys().copied().collect();
                if let Some(suggested) = suggest_command(&args[1], &commands) {
                    eprintln!("Did you mean: {} ?", suggested);
                }
                eprintln!("Use `help` for usage information.");
            }
        }
    } else {
        eprintln!("No arguments provided. Use `help` for usage information.");
    }
    print!("\n");

    Ok(())
}

use orinium_browser::engine::layouter::types::InfoNode;

/// Result of building layout for a page, keeping the DOM and JS runtime alive
/// so callers can dispatch clicks and inspect the mutated tree.
struct LayoutInfo {
    layout: LayoutNode,
    info: InfoNode,
}

fn build_layout_info(raw_url: &str, viewport: (f32, f32)) -> Result<LayoutInfo> {
    build_layout_info_inner(raw_url, viewport, 0)
}

fn build_layout_info_inner(
    raw_url: &str,
    viewport: (f32, f32),
    depth: usize,
) -> Result<LayoutInfo> {
    let parsed_url: url::Url = raw_url.parse()?;

    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
    let loader = BrowserResourceLoader::new(Some(Rc::new(net)));
    let resp = loader
        .fetch_blocking(parsed_url.clone())
        .expect("Failed to fetch URL");
    let html = String::from_utf8_lossy(&resp.body).to_string();

    let mut parser = HtmlParser::new(&html);
    let dom = Rc::new(parser.parse());

    // Run inline scripts so layout dumps reflect JS mutations.
    let mut js_runtime = JsRuntime::new(Rc::clone(&dom));
    js_runtime.set_document_url(parsed_url.as_str());
    for script in dom.collect_inline_scripts() {
        js_runtime.run_script(&script);
    }
    js_runtime.dispatch_dom_content_loaded();
    // Script-initiated top-level navigations (anti-bot challenge pages):
    // re-fetch the target URL and build the layout for the real page.
    if let Some(navigation) = js_runtime.take_navigation_requests().first().cloned()
        && depth < 4
    {
        let target = url::Url::parse(&navigation)
            .or_else(|_| parsed_url.join(&navigation))
            .expect("navigation URL");
        println!("Following script navigation to {target}");
        return build_layout_info_inner(target.as_str(), viewport, depth + 1);
    }

    let base_url = dom
        .find_all(|n| n.tag_name() == Some("base"))
        .iter()
        .filter_map(|node_ref| {
            let html_node = &node_ref.borrow().value;
            let href = html_node.get_attr("href")?;
            parsed_url.join(href).ok()
        })
        .next()
        .unwrap_or_else(|| parsed_url.clone());

    let style_links: Vec<url::Url> = dom
        .find_all(|n| n.tag_name() == Some("link"))
        .iter()
        .filter_map(|node_ref| {
            let node = node_ref.borrow();
            let html_node = &node.value;
            let rel = html_node.get_attr("rel")?;
            let href = html_node.get_attr("href")?;
            if rel == "stylesheet" {
                base_url.join(href).ok()
            } else {
                None
            }
        })
        .collect();

    let inline_styles = dom.collect_text_by_tag("style");

    let mut resolved_styles = ResolvedStyles::default();

    let ua_css = include_str!("../resource/user-agent.css");
    let ua_sheet = CssParser::new(ua_css)
        .parse()
        .expect("Failed to parse UA CSS");
    append_resolved_styles(
        &mut resolved_styles,
        CssResolver::resolve_with_origin(&ua_sheet, StyleOrigin::UserAgent),
    );

    for css in &inline_styles {
        if let Ok(sheet) = CssParser::new(css).parse() {
            append_resolved_styles(&mut resolved_styles, CssResolver::resolve(&sheet));
        }
    }

    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
    let css_loader = BrowserResourceLoader::new(Some(Rc::new(net)));
    for css_url in &style_links {
        println!("Fetching CSS: {}", css_url);
        if let Ok(css_resp) = css_loader.fetch_blocking(css_url.clone()) {
            let css = String::from_utf8_lossy(&css_resp.body).to_string();
            if let Ok(sheet) = CssParser::new(&css).parse() {
                append_resolved_styles(&mut resolved_styles, CssResolver::resolve(&sheet));
            }
        }
    }

    let measurer = PlatformTextMeasurer::new()
        .expect("Failed to initialize text measurer (no system font found)");
    let measurer = Arc::new(measurer);
    let (layout, info) = build_layout_and_info(
        &dom.root,
        &resolved_styles,
        measurer,
        InheritedCss {
            text_style: TextStyle::default(),
            text_flow_style: TextFlowStyle {
                font_size: 16.0,
                ..Default::default()
            },
            ..Default::default()
        },
        ElementChain::default(),
        dark_light::detect().map(Into::into).unwrap_or_else(|e| {
            log::error!("Failed to detect system color scheme, using default: {e}");
            Default::default()
        }),
        orinium_browser::engine::html::ScriptingMode::Enabled,
        viewport,
    );

    Ok(LayoutInfo { layout, info })
}

/// Builds layout from a live (post-JS-mutation) DOM and stylesheet bodies
/// that were already fetched by the webcompat harness, so the layout stage
/// does not re-fetch the page or its CSS. `<style>` blocks are re-collected
/// from the live DOM so scripts that inject styles are reflected.
fn build_layout_for_dom(
    dom: &Rc<DomTree>,
    external_css: &[String],
    viewport: (f32, f32),
) -> Result<LayoutInfo> {
    let mut resolved_styles = ResolvedStyles::default();

    let ua_css = include_str!("../resource/user-agent.css");
    let ua_sheet = CssParser::new(ua_css)
        .parse()
        .expect("Failed to parse UA CSS");
    append_resolved_styles(
        &mut resolved_styles,
        CssResolver::resolve_with_origin(&ua_sheet, StyleOrigin::UserAgent),
    );

    for css in dom.collect_text_by_tag("style") {
        if let Ok(sheet) = CssParser::new(&css).parse() {
            append_resolved_styles(&mut resolved_styles, CssResolver::resolve(&sheet));
        }
    }
    for css in external_css {
        if let Ok(sheet) = CssParser::new(css).parse() {
            append_resolved_styles(&mut resolved_styles, CssResolver::resolve(&sheet));
        }
    }

    let measurer = PlatformTextMeasurer::new()
        .expect("Failed to initialize text measurer (no system font found)");
    let measurer = Arc::new(measurer);
    let (layout, info) = build_layout_and_info(
        &dom.root,
        &resolved_styles,
        measurer,
        InheritedCss {
            text_style: TextStyle::default(),
            text_flow_style: TextFlowStyle {
                font_size: 16.0,
                ..Default::default()
            },
            ..Default::default()
        },
        ElementChain::default(),
        dark_light::detect().map(Into::into).unwrap_or_else(|e| {
            log::error!("Failed to detect system color scheme, using default: {e}");
            Default::default()
        }),
        orinium_browser::engine::html::ScriptingMode::Enabled,
        viewport,
    );

    Ok(LayoutInfo { layout, info })
}

/// JS prologue that intercepts `console.*` and records `warn`/`error` messages
/// (including the engine's own reports) so the harness can read them back.
/// Written to avoid engine-unsupported APIs: plain loops, array literals,
/// `push`, `join`, and `typeof` checks only.
const WEBCOMPAT_COLLECTOR: &str = r#"(function () {
  var errors = [];
  var levels = ["log", "warn", "error", "info", "debug"];
  for (var l = 0; l < levels.length; l++) {
    (function (level) {
      var orig = console[level];
      console[level] = function () {
        var msg = [];
        for (var i = 0; i < arguments.length; i++) {
          var a = arguments[i];
          try {
            if (typeof a === "string") msg.push(a);
            else if (a && a.message !== undefined) msg.push(String(a.message));
            else msg.push(String(a));
          } catch (e2) { msg.push("<unprintable>"); }
        }
        var text = msg.join(" ");
        if (level === "warn" || level === "error") errors.push("[" + level + "] " + text);
        try { orig.apply(console, arguments); } catch (e3) {}
      };
    })(levels[l]);
  }
  window.__orinium_errors = errors;
  window.__orinium_flush_errors = function () { return errors.join("\n"); };
})();"#;

struct WebcompatSubresource {
    label: &'static str,
    url: url::Url,
    status: u16,
    content_type: String,
    bytes: usize,
    preview: String,
}

/// A classic script in document order: inline text or an external URL
/// (resolved against the page base URL), plus its parse-time scheduling
/// attributes for diagnostics.
struct WebcompatScript {
    inline_source: Option<String>,
    resolved_url: Option<url::Url>,
    execution: ClassicScriptExecution,
}

/// True when `node` is nested inside a `<template>`; template contents are
/// inert until the template is cloned, so the scripts they hold must not run.
fn is_inside_template(node: &NodeRef<HtmlNodeType>) -> bool {
    let mut current = node.borrow().parent();
    while let Some(ancestor) = current {
        let tag = ancestor
            .borrow()
            .value
            .tag_name()
            .map(|tag| tag.to_ascii_lowercase());
        if tag.as_deref() == Some("template") {
            return true;
        }
        current = ancestor.borrow().parent();
    }
    false
}

fn push_subresource(
    label: &'static str,
    url: url::Url,
    status: u16,
    content_type: String,
    bytes: usize,
    preview: String,
    out: &mut Vec<WebcompatSubresource>,
) {
    out.push(WebcompatSubresource {
        label,
        url,
        status,
        content_type,
        bytes,
        preview,
    });
}

fn record_webcompat_fetch(
    loader: &BrowserResourceLoader,
    label: &'static str,
    url: url::Url,
    out: &mut Vec<WebcompatSubresource>,
) {
    match loader.fetch_blocking(url.clone()) {
        Ok(resp) => {
            let preview = String::from_utf8_lossy(&resp.body)
                .chars()
                .take(48)
                .collect::<String>()
                .replace('\n', " ");
            push_subresource(
                label,
                url,
                resp.status.as_u16(),
                resp.headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default(),
                resp.body.len(),
                preview,
                out,
            );
        }
        Err(e) => push_subresource(
            label,
            url,
            0,
            format!("ERROR: {e}"),
            0,
            String::new(),
            out,
        ),
    }
}

/// Diagnostic harness for how well this engine can load a real-world page.
/// Fetches the HTML, parses the DOM, discovers and fetches
/// scripts/stylesheets/fonts, runs the scripts through the JS engine
/// while capturing console errors and missing Web Platform APIs, and finally
/// attempts a full layout build. Used to drive the bsky.app web-tech effort.
fn run_webcompat(raw_url: &str) -> Result<()> {
    run_webcompat_depth(raw_url, 0)
}

/// `depth` bounds how many script-initiated navigations (anti-bot challenge
/// pages) the report will follow before giving up.
fn run_webcompat_depth(raw_url: &str, depth: usize) -> Result<()> {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let parsed_url: url::Url = raw_url.parse()?;
    println!(
        "{}",
        format!("Webcompat report for {parsed_url}")
            .bold()
            .underline()
    );

    // ---- Network stage ----
    println!("\n{}", "== Network ==".bold());
    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
    let loader = BrowserResourceLoader::new(Some(Rc::new(net)));

    let html_resp = match loader.fetch_blocking(parsed_url.clone()) {
        Ok(resp) => resp,
        Err(e) => {
            println!(
                "{}",
                format!("FATAL: could not fetch HTML: {e}").red().bold()
            );
            return Ok(());
        }
    };
    println!("  status: {}", html_resp.status.as_u16());
    println!("  status_text: {}", html_resp.status_text);
    for (key, value) in &html_resp.headers {
        println!("  header: {}: {}", key, value);
    }
    let html = String::from_utf8_lossy(&html_resp.body).to_string();
    println!("  body bytes: {}", html_resp.body.len());
    println!("  decoded chars: {}", html.chars().count());

    // ---- DOM stage ----
    println!("\n{}", "== DOM ==".bold());
    let mut parser = HtmlParser::new(&html);
    let dom = Rc::new(parser.parse());

    let mut node_count = 0usize;
    dom.traverse(&mut |_node_rc: &NodeRef<HtmlNodeType>| node_count += 1);
    let mut top_tags: Vec<String> = dom
        .root
        .borrow()
        .children()
        .iter()
        .filter_map(|child| child.borrow().value.tag_name().map(|tag| tag.to_string()))
        .collect();
    top_tags.sort();
    top_tags.dedup();
    println!("  DOM nodes: {}", node_count);
    println!("  top-level tags: {}", top_tags.join(", "));

    let base_url = dom
        .find_all(|n| n.tag_name() == Some("base"))
        .iter()
        .filter_map(|node_ref| {
            let html_node = &node_ref.borrow().value;
            let href = html_node.get_attr("href")?;
            parsed_url.join(href).ok()
        })
        .next()
        .unwrap_or_else(|| parsed_url.clone());

    // Classic scripts in document order (external + inline interleaved), so
    // `<script src="a.js"></script><script>…</script>` runs a.js before the
    // inline block — not external-first as earlier harness iterations did.
    let mut scripts: Vec<WebcompatScript> = Vec::new();
    let mut module_scripts = 0usize;
    let mut other_script_blocks = 0usize;
    for node_ref in dom.get_elements_by_tag_name("script") {
        if is_inside_template(&node_ref) {
            continue;
        }
        let (src, script_type, has_async, has_defer) = {
            let node = node_ref.borrow();
            let value = &node.value;
            (
                value.get_attr("src").map(str::to_string),
                value
                    .get_attr("type")
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase(),
                value.has_attr("async"),
                value.has_attr("defer"),
            )
        };
        if script_type == "module" {
            module_scripts += 1;
            continue;
        }
        if !script_type.is_empty()
            && script_type != "text/javascript"
            && script_type != "application/javascript"
        {
            other_script_blocks += 1;
            continue;
        }
        let execution = if has_async {
            ClassicScriptExecution::Async
        } else if has_defer {
            ClassicScriptExecution::Defer
        } else {
            ClassicScriptExecution::Default
        };
        if let Some(src) = src.filter(|src| !src.is_empty()) {
            if let Ok(url) = base_url.join(&src) {
                scripts.push(WebcompatScript {
                    inline_source: None,
                    resolved_url: Some(url),
                    execution,
                });
            }
        } else {
            scripts.push(WebcompatScript {
                inline_source: Some(DomTree::inner_text(&node_ref)),
                resolved_url: None,
                execution: ClassicScriptExecution::Default,
            });
        }
    }

    let mut style_urls: Vec<url::Url> = Vec::new();
    let mut font_urls: Vec<url::Url> = Vec::new();
    let mut preload_script_urls: Vec<url::Url> = Vec::new();
    for node_ref in dom.find_all(|n| n.tag_name() == Some("link")) {
        let html_node = &node_ref.borrow().value;
        let rel = html_node.get_attr("rel").unwrap_or("");
        let as_kind = html_node.get_attr("as").unwrap_or("");
        let Some(href) = html_node.get_attr("href") else {
            continue;
        };
        let Ok(url) = base_url.join(href) else {
            continue;
        };
        match rel {
            "stylesheet" => style_urls.push(url),
            "preload" => match as_kind {
                "font" => font_urls.push(url),
                "script" => preload_script_urls.push(url),
                "style" => style_urls.push(url),
                _ => {}
            },
            _ => {}
        }
    }
    // Preload hints may reference scripts that are not otherwise in the DOM;
    // append them (deduped, in hint order) so they are still exercised.
    for url in preload_script_urls {
        if !scripts
            .iter()
            .any(|script| script.resolved_url.as_ref() == Some(&url))
        {
            scripts.push(WebcompatScript {
                inline_source: None,
                resolved_url: Some(url),
                execution: ClassicScriptExecution::Default,
            });
        }
    }

    // ---- Subresource stage ----
    println!("\n{}", "== Subresources ==".bold());
    const SUBRESOURCE_CAP: usize = 48;

    let script_urls: Vec<url::Url> = scripts
        .iter()
        .filter_map(|script| script.resolved_url.clone())
        .collect();
    let tested_scripts = script_urls.len().min(SUBRESOURCE_CAP);
    let tested_styles = style_urls.len().min(SUBRESOURCE_CAP);
    let tested_fonts = font_urls.len().min(SUBRESOURCE_CAP);
    println!(
        "  stylesheets: {} discovered, {} tested",
        style_urls.len(),
        tested_styles
    );
    println!(
        "  classic scripts: {} discovered ({} external, {} inline), {} external tested",
        scripts.len(),
        script_urls.len(),
        scripts.len() - script_urls.len(),
        tested_scripts
    );
    println!(
        "  preloaded fonts: {} discovered, {} tested",
        font_urls.len(),
        tested_fonts
    );
    println!("  module scripts: {module_scripts} (not executed here)");
    if other_script_blocks > 0 {
        println!("  other script blocks: {other_script_blocks} (data / non-JS type)");
    }
    let defer_count = scripts
        .iter()
        .filter(|script| script.execution == ClassicScriptExecution::Defer)
        .count();
    let async_count = scripts
        .iter()
        .filter(|script| script.execution == ClassicScriptExecution::Async)
        .count();
    if defer_count + async_count > 0 {
        println!(
            "  scheduling: defer: {defer_count}, async: {async_count} (executed in document order)"
        );
    }

    // Fetch each discovered subresource exactly once; the JavaScript and
    // layout stages reuse these bodies instead of hammering the network.
    let mut script_bodies: HashMap<url::Url, Result<String, String>> = HashMap::new();
    let mut css_bodies: Vec<String> = Vec::new();
    let mut subresources: Vec<WebcompatSubresource> = Vec::new();
    for url in script_urls.iter().take(SUBRESOURCE_CAP).cloned() {
        match loader.fetch_blocking(url.clone()) {
            Ok(resp) => {
                let preview = String::from_utf8_lossy(&resp.body)
                    .chars()
                    .take(48)
                    .collect::<String>()
                    .replace('\n', " ");
                push_subresource(
                    "script",
                    url.clone(),
                    resp.status.as_u16(),
                    resp.headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default(),
                    resp.body.len(),
                    preview,
                    &mut subresources,
                );
                let body = String::from_utf8_lossy(&resp.body).to_string();
                script_bodies.insert(url, Ok(body));
            }
            Err(e) => {
                push_subresource(
                    "script",
                    url.clone(),
                    0,
                    format!("ERROR: {e}"),
                    0,
                    String::new(),
                    &mut subresources,
                );
                script_bodies.insert(url, Err(e.to_string()));
            }
        }
    }
    for url in style_urls.iter().take(SUBRESOURCE_CAP).cloned() {
        match loader.fetch_blocking(url.clone()) {
            Ok(resp) => {
                let preview = String::from_utf8_lossy(&resp.body)
                    .chars()
                    .take(48)
                    .collect::<String>()
                    .replace('\n', " ");
                push_subresource(
                    "css",
                    url.clone(),
                    resp.status.as_u16(),
                    resp.headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default(),
                    resp.body.len(),
                    preview,
                    &mut subresources,
                );
                css_bodies.push(String::from_utf8_lossy(&resp.body).to_string());
            }
            Err(e) => {
                push_subresource(
                    "css",
                    url,
                    0,
                    format!("ERROR: {e}"),
                    0,
                    String::new(),
                    &mut subresources,
                );
            }
        }
    }
    for url in font_urls.iter().take(SUBRESOURCE_CAP).cloned() {
        record_webcompat_fetch(&loader, "font", url, &mut subresources);
    }
    let mut failed = 0;
    for s in &subresources {
        if s.status != 200 {
            failed += 1;
        }
        let status = if s.status == 200 {
            s.status.to_string().green().to_string()
        } else {
            s.status.to_string().red().to_string()
        };
        println!(
            "  [{:>6}] {:>4} {:>15} {:>8} bytes {}",
            s.label, status, s.content_type, s.bytes, s.url
        );
        if !s.preview.is_empty() {
            println!("           \"{}\"", s.preview);
        }
    }
    println!("  subresource failures: {failed}");

    // ---- JavaScript stage ----
    println!("\n{}", "== JavaScript ==".bold());
    let mut js = JsRuntime::new(Rc::clone(&dom));
    js.set_document_url(parsed_url.as_str());
    js.set_page_origin(&format!(
        "{}://{}",
        parsed_url.scheme(),
        parsed_url.host_str().unwrap_or("")
    ));
    js.run_script(WEBCOMPAT_COLLECTOR);
    missing_api_detector::install_missing_api_report(&mut js);

    // Classic script execution follows document order: each `<script>` (inline
    // or external) runs when its position in the source is reached, matching
    // the browser's parse-time scheduling instead of running all externals
    // before all inline blocks.
    let mut ran_scripts = 0usize;
    let mut fetch_failed_scripts = 0usize;
    let mut untested_scripts = 0usize;
    for script in &scripts {
        let source = match (&script.inline_source, &script.resolved_url) {
            (Some(source), _) => source.clone(),
            (None, Some(url)) => match script_bodies.get(url) {
                Some(Ok(body)) => body.clone(),
                Some(Err(_)) => {
                    fetch_failed_scripts += 1;
                    continue;
                }
                None => {
                    untested_scripts += 1;
                    continue;
                }
            },
            (None, None) => continue,
        };
        ran_scripts += 1;
        js.run_script(&format!(
            "try {{\n{}\n}} catch (e) {{ console.error('[uncaught]', e && e.stack || String(e)); }}",
            source
        ));
    }
    js.dispatch_dom_content_loaded();
    js.dispatch_window_load();
    for _ in 0..60 {
        js.run_due_timers();
    }
    let navigations = js.take_navigation_requests();
    println!(
        "  scripts ran: {} ({} failed to fetch, {} untested)",
        ran_scripts, fetch_failed_scripts, untested_scripts
    );
    if navigations.is_empty() {
        println!("  no script-initiated navigation");
    } else {
        println!("  script-initiated navigations: {}", navigations.join(", "));
    }

    // Script-initiated top-level navigations (anti-bot challenge pages):
    // re-fetch the target URL once and report on the real page.
    if let Some(navigation) = navigations.first().cloned()
        && depth < 4
    {
        let target = url::Url::parse(&navigation)
            .or_else(|_| parsed_url.join(&navigation))
            .expect("navigation URL");
        println!("Following script navigation to {target}");
        return run_webcompat_depth(target.as_str(), depth + 1);
    }

    // ---- CSS usage census (post-script live DOM) ----
    println!("\n{}", "== CSS census (post-script DOM) ==".bold());
    let css_census = js
        .eval_value(
            r#"
(function () {
    var out = {};
    out.nodes = document.querySelectorAll('*').length;
    out.bodyChildren = document.body ? document.body.children.length : 0;
    out.bodyHtml = document.body ? document.body.innerHTML.slice(0, 400) : '(no body)';
    out.rootIds = [];
    var root = document.body && document.body.firstChild;
    for (var r = 0; r < 5 && root; r++, root = root.nextSibling) {
        out.rootIds.push(String(root.id || '') + ':' + (root.className || '').toString().slice(0, 40));
    }
    var styles = {};
    var samples = [];
    var els = document.querySelectorAll('*');
    for (var i = 0; i < els.length && i < 400; i++) {
        var st = els[i].getAttribute && els[i].getAttribute('style');
        if (st) {
            var parts = st.split(';');
            for (var j = 0; j < parts.length; j++) {
                var kv = parts[j].split(':');
                if (kv.length == 2) {
                    var k = kv[0].trim();
                    styles[k] = (styles[k] || 0) + 1;
                }
            }
            if (samples.length < 6) samples.push(els[i].tagName.toLowerCase() + ':' + st.slice(0, 120));
        }
    }
    out.inlineStyles = styles;
    out.inlineSamples = samples;
    var gcs = null;
    try {
        var e = els[0] || document.body;
        var cs = getComputedStyle(e);
        gcs = {};
        gcs.display = cs.getPropertyValue('display');
        gcs.width = cs.getPropertyValue('width');
        gcs.height = cs.getPropertyValue('height');
        gcs.opacity = cs.getPropertyValue('opacity');
        gcs.isInlineDecl = String(cs).slice(0, 40);
    } catch (err) {
        gcs = 'ERR:' + String(err);
    }
    out.computedSample = gcs;
    try {
        var mm = matchMedia('(max-width: 600px)');
        out.matchMedia = { matches: mm.matches, media: mm.media };
    } catch (err) {
        out.matchMedia = 'ERR:' + String(err);
    }
    return JSON.stringify(out);
})()
"#,
        )
        .to_console_string();
    println!("  {css_census}");

    let console_report = js
        .eval_value(
            "window.__orinium_flush_errors ? window.__orinium_flush_errors() : '(collector not installed)'",
        )
        .to_console_string();

    let mut missing: Vec<String> = Vec::new();
    for line in console_report.lines() {
        if let Some(names) = line.strip_prefix("[warn] [missing-api] ") {
            missing.extend(names.split(',').map(|s| s.trim().to_string()));
        }
    }
    missing.sort();
    missing.dedup();

    println!("\n{}", "== Captured console ==".bold());
    if console_report.trim().is_empty() {
        println!("  (no console.warn/error captured)");
    } else {
        for line in console_report.lines().take(80) {
            println!("  {line}");
        }
    }

    println!(
        "\n{}",
        format!("== Missing Web Platform APIs ({}) ==", missing.len()).bold()
    );
    for name in &missing {
        println!("  - {name}");
    }

    // ---- Layout smoke test ----
    println!("\n{}", "== Layout ==".bold());
    // Layout builds from the same live DOM (post-JS mutations) and the
    // already-fetched stylesheet bodies — nothing is re-fetched here.
    let layout_result = catch_unwind(AssertUnwindSafe(|| {
        build_layout_for_dom(&dom, &css_bodies, (800.0, 600.0))
    }));
    match layout_result {
        Ok(Ok(ctx)) => {
            let dump_len = format!("{:#?}", ctx.info).len();
            println!(
                "  layout: OK (InfoNode dump {} chars)",
                dump_len.to_string().green()
            );
        }
        Ok(Err(e)) => println!("  {} setup error: {e}", "layout:".red().bold()),
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            println!("  layout: {} {msg}", "PANIC".red().bold());
        }
    }

    Ok(())
}

/// Reads the optional viewport dimensions (`WIDTH HEIGHT`) from `args`,
/// falling back to `default_w`/`default_h` when they are omitted.
fn viewport_args(args: &[String], default_w: f32, default_h: f32) -> Result<(f32, f32)> {
    if args.len() >= 5 {
        Ok((args[3].parse()?, args[4].parse()?))
    } else {
        Ok((default_w, default_h))
    }
}

use strsim::levenshtein;

fn suggest_command<'a>(input: &'a str, commands: &'a [&'a str]) -> Option<&'a str> {
    commands
        .iter()
        .min_by_key(|cmd| levenshtein(input, cmd))
        .and_then(|&cmd| {
            if levenshtein(input, cmd) <= 4 {
                // Suggest if edit distance is within 4
                Some(cmd)
            } else {
                None
            }
        })
}

use std::collections::HashMap;

#[rustfmt::skip]
fn get_commands<'a>() -> HashMap<&'a str, (&'a str, &'a str, &'a str)> {
    let mut map = HashMap::new();

    map.insert(
        "parse_dom",
        (
            "Fetch and parse the HTML of the given URL into a DOM tree. Optionally hide specified tag names and their attributes.",
            "URL [..]",
            "If additional arguments are provided, the second argument is a comma-separated list of tag names to hide, and the third argument is a boolean (true/false) indicating whether to hide attributes of those tags."
        ),
    );
    map.insert(
        "parse_cssom",
        (
            "Fetch and parse the CSS of the given URL into a CSSOM tree.",
            "URL",
            "",
        ),
    );
    map.insert(
        "send_request",
        (
            "Send a basic HTTP/HTTPS request (no redirect handling).",
            "URL",
            "",
        ),
    );
    map.insert(
        "fetch_url",
        (
            "Fetch a URL and display status, headers, and body.",
            "URL",
            "",
        ),
    );
    map.insert(
        "dump_infonode",
        (
            "Fetch HTML and CSS, build layout tree, and dump the InfoNode (render info) tree.",
            "URL",
            "",
        ),
    );
    map.insert(
        "dump_domnode",
        (
            "Fetch HTML, run inline scripts, and dump the DOM tree.",
            "URL",
            "",
        ),
    );
    map.insert(
        "dump_layoutnode",
        (
            "Fetch HTML and CSS, build layout tree, run layout engine, and dump the LayoutNode tree.",
            "URL [WIDTH HEIGHT]",
            "The optional WIDTH HEIGHT specify the layout viewport size; it defaults to 800x600.",
        ),
    );
    map.insert(
        "dump_draw_command",
        (
            "Fetch HTML and CSS, build layout tree, run layout engine, and generate draw commands.",
            "URL [WIDTH HEIGHT]",
            "The optional WIDTH HEIGHT specify the layout viewport size; it defaults to 800x600.",
        ),
    );
    map.insert(
        "run_layout",
        (
            "Fetch HTML and CSS, build layout tree, and run layout engine.",
            "URL",
            ""
        ),
    );
    map.insert(
        "simple_render",
        (
            "Fetch HTML and CSS, build layout tree, generate draw commands, then render.",
            "URL",
            "",
        ),
    );
    map.insert(
        "webcompat",
        (
            "Diagnostic report of how well this engine can load a real-world URL.",
            "URL",
            "Fetches the HTML, parses the DOM, discovers and fetches scripts/stylesheets/fonts, runs the scripts through the JS engine while capturing console errors and missing Web Platform APIs, and attempts a full layout build.",
        ),
    );

    map
}
