use orinium_browser::{
    browser::{BrowserApp, Tab},
    engine::html::parser::Parser as HtmlParser,
    platform::network::NetworkCore,
};

use colored::*;

use anyhow::Result;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect::<Vec<String>>();
    if args.len() >= 2 {
        match args[1].as_str() {
            "help" => {
                let commands = get_commands();
                println!("{}", "Orinium Browser Test Application".bold().underline());
                println!("\n{}", "Usage:".bold());
                println!("  cargo run --example tests [COMMAND] [ARGS]\n");

                println!("{}", "Available Commands:".bold());
                for (name, (description, args)) in &commands {
                    println!(
                        "  {:<15} {:<4} - {}",
                        name.green().bold(),
                        args.cyan(),
                        description
                    );
                }

                println!("\n{}", "Note:".bold());
                println!("  - URLs must include the scheme (http:// or https://).");
                println!("  - For 'plain_css_parse', the CSS string must be quoted.");
            }
            "parse_dom" => {
                if args.len() == 3 {
                    let url = &args[2];
                    println!("Parsing DOM for URL: {}", url);
                    let net = NetworkCore::new();
                    let resp = net.fetch_url(url).await.expect("Failed to fetch URL");
                    let html = String::from_utf8_lossy(&resp.body).to_string();
                    println!(
                        "Fetched HTML (first 50 chars):\n{}",
                        html.chars().take(50).collect::<String>()
                    );
                    let mut parser = HtmlParser::new(&html);
                    let dom = parser.parse();
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
                    let resp = net.fetch_url(url).await.expect("Failed to fetch URL");
                    let css = String::from_utf8_lossy(&resp.body).to_string();
                    println!(
                        "Fetched CSS (first 50 chars):\n{}",
                        css.chars().take(50).collect::<String>()
                    );
                    let mut parser = orinium_browser::engine::css::cssom::parser::Parser::new(&css);
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
                    let mut parser = orinium_browser::engine::css::cssom::parser::Parser::new(css);
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
                    match net.send_request(url, hyper::Method::GET).await {
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
                    match net.fetch_url(url).await {
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

                    let net = browser.network();

                    let mut tab = Tab::new(net);
                    tab.load_from_url(&url).await?;

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
fn get_commands<'a>() -> HashMap<&'a str, (&'a str, &'a str)> {
    let mut map = HashMap::new();

    map.insert(
        "parse_dom",
        (
            "Fetch and parse the HTML of the given URL into a DOM tree.",
            "[URL]",
        ),
    );
    map.insert(
        "parse_cssom",
        (
            "Fetch and parse the CSS of the given URL into a CSSOM tree.",
            "[URL]",
        ),
    );
    map.insert(
        "plain_css_parse",
        (
            "Parse a CSS string directly into a CSSOM tree.",
            "[CSS]",
        ),
    );
    map.insert(
        "send_request",
        (
            "Send a basic HTTP/HTTPS request (no redirect handling).",
            "[URL]",
        ),
    );
    map.insert(
        "fetch_url",
        (
            "Fetch a URL and display status, headers, and body.",
            "[URL]",
        ),
    );
    map.insert(
        "simple_render",
        (
            "Fetch HTML from a URL, parse DOM and CSSOM, and generate draw commands.",
            "[URL]",
        ),
    );

    map
}
