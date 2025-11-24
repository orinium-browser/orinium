use orinium_browser::{
    browser::BrowserApp,
    engine::html::parser::Parser as HtmlParser,
    platform::{network::NetworkCore, system::App},
    renderer::RenderTree,
};

use colored::*;

use anyhow::Result;
use std::env;
use winit::event_loop::EventLoop;

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
                    let net = NetworkCore::new();
                    let resp = net.fetch_url(url).await.expect("Failed to fetch URL");
                    let html = String::from_utf8_lossy(&resp.body).to_string();
                    let mut html_parser = HtmlParser::new(&html);
                    let dom = html_parser.parse();
                    /*
                    let css = "";
                    let mut css_parser = CssParser::new(css);
                    let cssom = css_parser.parse()?;
                    */
                    let mut style_tree =
                        orinium_browser::engine::styler::StyleTree::transform(&dom);
                    style_tree.style(&[]);
                    let computed_tree = style_tree.compute();
                    let mut render_tree = RenderTree::from_computed_tree(&computed_tree);
                    // レンダラーを作成して描画命令を生成
                    let renderer = orinium_browser::engine::renderer::Renderer::new();
                    let draw_commands = renderer.generate_draw_commands(&mut render_tree);
                    println!("dom_tree: {}", dom);
                    println!("style_tree: {}" ,style_tree);
                    // println!("computed_tree: {}", computed_tree);
                    println!("render_tree: {}", render_tree);
                    // ウィンドウとイベントループを作成
                    let event_loop =
                        EventLoop::<orinium_browser::platform::system::State>::with_user_event()
                            .build()?;
                    let mut app = App::new(BrowserApp::new().with_draw_commands(draw_commands));
                    let _ = event_loop.run_app(&mut app);
                } else {
                    eprintln!("Please provide a URL for simple rendering test.");
                }
            }
            _ => {
                eprintln!("Unknown argument: {}", args[1]);
                eprintln!("Use help for usage information.");
            }
        }
    } else {
        eprintln!("No arguments provided. Use help for usage information.");
    }
    print!("\n");

    Ok(())
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
