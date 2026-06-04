use orinium_browser::{
    browser::{BrowserApp, Tab, core::resource_loader::BrowserResourceLoader},
    engine::{
        css::parser::Parser as CssParser,
        html::{HtmlNodeType, parser::Parser as HtmlParser},
        layouter::{
            InheritedCss, build_layout_and_info,
            css_resolver::{CssResolver, ResolvedStyles},
            types::TextStyle,
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
use std::{env, rc::Rc};
use ui_layout::{LayoutEngine, LayoutNode};

fn main() -> Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    env_logger::init();

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
                    println!("  - URLs must include the scheme (http:// or https://).");

                    println!("\nTo see more details about a specific command, run:");
                    println!("  cargo run --example tests help [COMMAND]");
                }
            }
            "parse_dom" => {
                if args.len() == 3 || args.len() == 4 || args.len() == 5 {
                    let url = &args[2];
                    println!("Parsing DOM for URL: {}", url);
                    let net = NetworkCore::new();
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
                    let net = NetworkCore::new();
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
                    let net = NetworkCore::new();
                    net.set_network_config(NetworkConfig {
                        follow_redirects: false,
                        ..Default::default()
                    });
                    match net.fetch_blocking(url) {
                        Ok(resp) => {
                            println!("Response Status: {}", resp.status);
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
                    let net = NetworkCore::new();
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

                    let (_layout, info) = build_layout_info(raw_url)?;

                    println!("\nInfoNode:\n{:#?}", info);
                } else {
                    eprintln!("Please provide a URL for dump_infonode test.");
                }
            }
            "dump_layoutnode" => {
                if args.len() == 3 {
                    let raw_url = &args[2];
                    println!("Dumping LayoutNode for URL: {}", raw_url);

                    let (mut layout, _info) = build_layout_info(raw_url)?;
                    LayoutEngine::layout(&mut layout, 800.0, 600.0);

                    println!("\nLayoutNode:\n{:#?}", layout);
                } else {
                    eprintln!("Please provide a URL for dump_layoutnode test.");
                }
            }
            "dump_draw_command" => {
                if args.len() == 3 {
                    let raw_url = &args[2];
                    println!("Dumping draw commands for URL: {}", raw_url);

                    let (mut layout, info) = build_layout_info(raw_url)?;
                    LayoutEngine::layout(&mut layout, 800.0, 600.0);

                    let mut draw_commands = Vec::new();
                    generate_draw_commands(&mut draw_commands, &layout, &info);

                    println!("\nGenerated {} draw commands:", draw_commands.len());
                    for (i, cmd) in draw_commands.iter().enumerate() {
                        println!("  [{:>3}] {:?}", i, cmd);
                    }
                } else {
                    eprintln!("Please provide a URL for dump draw commands test.");
                }
            }
            "simple_render" => {
                if args.len() == 3 {
                    let url = &args[2];
                    println!("Testing simple rendering for URL: {}", url);

                    let mut browser = BrowserApp::default();

                    let mut tab = Tab::new();
                    tab.navigate(url.parse()?);

                    browser.add_tab(tab);

                    browser.run()?
                } else {
                    eprintln!("Please provide a URL for simple rendering test.");
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

fn build_layout_info(raw_url: &str) -> Result<(LayoutNode, InfoNode)> {
    let parsed_url: url::Url = raw_url.parse()?;

    let net = NetworkCore::new();
    let loader = BrowserResourceLoader::new(Some(Rc::new(net)));
    let resp = loader
        .fetch_blocking(parsed_url.clone())
        .expect("Failed to fetch URL");
    let html = String::from_utf8_lossy(&resp.body).to_string();

    let mut parser = HtmlParser::new(&html);
    let dom = parser.parse();

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
    resolved_styles.extend(CssResolver::resolve(&ua_sheet));

    for css in &inline_styles {
        if let Ok(sheet) = CssParser::new(css).parse() {
            resolved_styles.extend(CssResolver::resolve(&sheet));
        }
    }

    let css_loader = BrowserResourceLoader::new(Some(Rc::new(NetworkCore::new())));
    for css_url in &style_links {
        println!("Fetching CSS: {}", css_url);
        if let Ok(css_resp) = css_loader.fetch_blocking(css_url.clone()) {
            let css = String::from_utf8_lossy(&css_resp.body).to_string();
            if let Ok(sheet) = CssParser::new(&css).parse() {
                resolved_styles.extend(CssResolver::resolve(&sheet));
            }
        }
    }

    let measurer = PlatformTextMeasurer::new()
        .expect("Failed to initialize text measurer (no system font found)");
    let (layout, info) = build_layout_and_info(
        &dom.root,
        &resolved_styles,
        &measurer,
        InheritedCss {
            text_style: TextStyle {
                font_size: 16.0,
                ..Default::default()
            },
        },
        Vec::new(),
    );

    Ok((layout, info))
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
        "dump_layoutnode",
        (
            "Fetch HTML and CSS, build layout tree, run layout engine, and dump the LayoutNode tree.",
            "URL",
            "",
        ),
    );
    map.insert(
        "dump_draw_command",
        (
            "Fetch HTML and CSS, build layout tree, run layout engine, and generate draw commands.",
            "URL",
            "",
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

    map
}
