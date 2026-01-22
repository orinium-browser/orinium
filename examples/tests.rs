use orinium_browser::{
    browser::{BrowserApp, Tab, core::resource_loader::BrowserResourceLoader},
    engine::html::parser::Parser as HtmlParser,
    html::HtmlNodeType,
    network::NetworkConfig,
    platform::network::NetworkCore,
};

use colored::*;

use anyhow::Result;
use std::{env, sync::Arc};

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect::<Vec<String>>();
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
                    println!("  - For 'plain_css_parse', the CSS string must be quoted.");

                    println!("\nTo see more details about a specific command, run:");
                    println!("  cargo run --example tests help [COMMAND]");
                }
            }
            "parse_dom" => {
                if args.len() == 3 || args.len() == 4 || args.len() == 5 {
                    let url = &args[2];
                    println!("Parsing DOM for URL: {}", url);
                    let net = NetworkCore::new();
                    let loader = BrowserResourceLoader::new(Some(Arc::new(net)));
                    let resp = loader
                        .fetch_blocking(url.clone())
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

                        dom.traverse(&mut |n| {
                            let mut node = n.borrow_mut();

                            if hide_tag_names.iter().any(|hide| {
                                hide == &node
                                    .value
                                    .tag_name()
                                    .unwrap_or("".to_string())
                                    .to_ascii_lowercase()
                            }) {
                                node.children_mut().clear();
                            }

                            if let HtmlNodeType::Element { attributes, .. } = &mut node.value
                                && hidden_attr
                            {
                                attributes.clear();
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
                    let loader = BrowserResourceLoader::new(Some(Arc::new(net)));
                    let resp = loader
                        .fetch_blocking(url.clone())
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
            "plain_css_parse" => {
                if args.len() == 3 {
                    let css = &args[2];
                    println!("Parsing plain CSS:\n{}", css);
                    let mut parser = orinium_browser::engine::css::parser::Parser::new(css);
                    let cssom = parser.parse()?;
                    println!("CSSOM Tree:\n{}", cssom);
                } else {
                    eprintln!("Please provide a CSS string for plain CSS parsing test.");
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
                            let body_str = String::from_utf8_lossy(&resp.body);
                            println!("{}", body_str);
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
                            println!("Response Reason_phrase: {}", resp.reason_phrase);
                            println!("Response Headers:");
                            for (key, value) in &resp.headers {
                                println!("{}: {}", key, value);
                            }
                            println!("Response Body:");
                            let body_str = String::from_utf8_lossy(&resp.body);
                            println!("{}", body_str);
                        }
                        Err(e) => {
                            eprintln!("Failed to fetch URL: {}", e);
                        }
                    }
                } else {
                    eprintln!("Please provide a URL for fetching test.");
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
                eprintln!("Use help for usage information.");
            }
        }
    } else {
        eprintln!("No arguments provided. Use help for usage information.");
    }
    print!("\n");

    Ok(())
}

use strsim::levenshtein;

fn suggest_command<'a>(input: &'a str, commands: &'a [&'a str]) -> Option<&'a str> {
    commands
        .iter()
        .min_by_key(|cmd| levenshtein(input, cmd))
        .and_then(|&cmd| {
            if levenshtein(input, cmd) <= 4 {
                // 文字数差が4以内なら提案
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
        "plain_css_parse",
        (
            "Parse a CSS string directly into a CSSOM tree.",
            "RAW_CSS",
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
        "simple_render",
        (
            "Fetch HTML from a URL, parse DOM and CSSOM, and generate draw commands.",
            "URL",
            "",
        ),
    );

    map
}
