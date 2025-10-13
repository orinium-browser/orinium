use orinium_browser::{engine::html::parser, platform::network::NetworkCore, platform::ui::App};

use std::env;
use winit::event_loop::EventLoop;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect::<Vec<String>>();
    if args.len() >= 2 {
        match args[1].as_str() {
            "help" => {
                println!("This is a test application for Orinium Browser development.");
                println!("Usage: cargo run --example tests [NAME]\n");
                println!("Test names:");
                println!("create_window - Create a window and display it.");
                println!("parse_dom [URL] - Test DOM parsing functionality.");
                println!("fetch_url [URL] - Test network fetching functionality.");
                println!("help - Show this help message.");
            }
            "create_window" => {
                if let Err(e) = run() {
                    eprintln!("Failed to create window: {e:?}");
                }
            }
            "parse_dom" => {
                if args.len() == 3 {
                    let url = &args[2];
                    println!("Parsing DOM for URL: {}", url);
                    let net = NetworkCore::new().unwrap();
                    let resp = net.fetch(url).await.expect("Failed to fetch URL");
                    let html = String::from_utf8_lossy(&resp.body).to_string();
                    println!(
                        "Fetched HTML (first 50 chars):\n{}",
                        html.chars().take(50).collect::<String>()
                    );
                    let mut parser = parser::Parser::new(&html);
                    let dom = parser.parse();
                    println!("DOM Tree:\n{}", dom.borrow());
                } else {
                    eprintln!("Please provide a URL for DOM parsing test.");
                }
            }
            "fetch_url" => {
                if args.len() == 3 {
                    let url = &args[2];
                    println!("Fetching URL: {}", url);
                    let net = NetworkCore::new().unwrap();
                    match net.fetch(url).await {
                        Ok(resp) => {
                            println!("Response Reason_phrase: {}", resp.reason_phrase);
                            println!("Response Headers:");
                            for (key, value) in &resp.headers {
                                println!("{}: {}", key, value);
                            }
                            println!("Response Body (first 100 chars):");
                            let body_str = String::from_utf8_lossy(&resp.body);
                            println!("{}", &body_str[..100.min(body_str.len())]);
                        }
                        Err(e) => {
                            eprintln!("Failed to fetch URL: {}", e);
                        }
                    }
                } else {
                    eprintln!("Please provide a URL for fetching test.");
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
}

fn run() -> anyhow::Result<()> {
    env_logger::init();

    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = App::new();
    event_loop.run_app(&mut app)?;

    Ok(())
}
