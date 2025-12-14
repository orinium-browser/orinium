use anyhow::Result;
use orinium_browser::browser::{BrowserApp, Tab};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let html = r#"
        <!DOCTYPE html>
        <html>
            <head>
                <title>Text Clip Example</title>
                <style>
                    .clip {
                        width: 300px;
                        height: 120px;
                        overflow: auto;
                        border: 1px solid black;
                    }
                </style>
            </head>
            <body>
                <h1>Text Clip Demo</h1>
                <div class="clip">
                    This is a very long sample text that should be clipped by the container's box.
                    Lorem ipsum dolor sit amet, consectetur adipiscing elit. Vivamus lacinia odio vitae vestibulum vestibulum. Cras venenatis euismod malesuada.
                </div>
            </body>
        </html>
    "#;

    let mut browser = BrowserApp::default();

    let mut tab = Tab::new(browser.network());
    tab.load_from_raw_html(html);

    browser.add_tab(tab);

    browser.run()?;

    Ok(())
}
