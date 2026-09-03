use orinium_browser::{
    ProcessHandler,
    browser::core::resource_loader::BrowserResourceLoader,
    engine::{html::parser::Parser as HtmlParser, js::JsRuntime},
    platform::network::NetworkCore,
};

use anyhow::Result;
use std::{env, rc::Rc};

const ACID3_URL: &str = "http://acid3.acidtests.org/";

fn main() -> Result<()> {
    if let Some(handler) = ProcessHandler::current() {
        handler.handle();
    }

    if env::var("RUST_LOG").is_err() {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    } else {
        env_logger::init();
    }

    run_acid3(ACID3_URL)
}

/// Runs the Acid3 harness headlessly: fetch, parse, execute inline scripts,
/// drive the `update()`-driven test loop, and report the final score. This is
/// the objective measure used to gauge Acid3 support without a GUI.
fn run_acid3(raw_url: &str) -> Result<()> {
    let parsed_url: url::Url = raw_url.parse()?;
    let net = NetworkCore::new().expect("Failed to create NetworkCore instansce");
    let loader = BrowserResourceLoader::new(Some(Rc::new(net)));
    let resp = loader
        .fetch_blocking(parsed_url.clone())
        .expect("Failed to fetch URL");
    let html = String::from_utf8_lossy(&resp.body).to_string();

    let mut parser = HtmlParser::new(&html);
    let dom = Rc::new(parser.parse());

    let mut js = JsRuntime::new(Rc::clone(&dom));
    js.set_document_url(raw_url);
    js.set_page_origin(&parsed_url.origin().ascii_serialization());

    // Execute all inline scripts (the main Acid3 harness among them).
    for script in dom.collect_inline_scripts() {
        js.run_script(&script);
    }

    // Confirm the harness globals are present before we drive the loop.
    let total = js.eval_value("tests.length").to_number();

    // Fire the window `load` (equivalent to `<body onload="update()">`),
    // which starts the timer-driven test loop.
    js.dispatch_window_load();

    // Pump timers, mirroring the GUI frame loop. The harness schedules the
    // next test via setTimeout(update, 10), and some tests request retries.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let mut last_score = -1.0;
    let mut stalls = 0;
    loop {
        let score = js.eval_value("score").to_number();
        if score != last_score {
            last_score = score;
            stalls = 0;
        } else {
            stalls += 1;
        }
        // Stop when the harness reports completion or we've been idle (no new
        // test for 2s) or the hard deadline expires.
        if js.eval_value("index >= tests.length").as_boolean() == Some(true) {
            break;
        }
        if stalls > 200 || std::time::Instant::now() >= deadline {
            break;
        }
        js.run_due_timers();

        // Drain any `<iframe src>` loads requested by JS (e.g. Acid3 bucket 5).
        // Each is fetched, parsed, installed as the iframe's contentDocument,
        // and its `load` event fired.
        for req in std::mem::take(&mut js.take_iframe_fetch_requests()) {
            match req.url.parse::<url::Url>() {
                Ok(url) if url.scheme() == "http" || url.scheme() == "https" => {
                    match loader.fetch_blocking(url) {
                        Ok(resp) => {
                            let html = String::from_utf8_lossy(&resp.body).to_string();
                            js.resolve_iframe_fetch(req.dom_id, html);
                        }
                        Err(_) => js.reject_iframe_fetch(req.dom_id),
                    }
                }
                _ => js.reject_iframe_fetch(req.dom_id),
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let finalscore = js.eval_value("score").to_number();
    let errors = js.eval_value("errors").to_number();
    let log = js.eval_value("log").to_string();
    println!("==================================================");
    println!(
        "ACID3 SCORE: {}/{}  (errors: {})",
        finalscore, total, errors
    );
    println!("==================================================");
    if !log.is_empty() {
        println!("{}", log);
    }
    Ok(())
}
