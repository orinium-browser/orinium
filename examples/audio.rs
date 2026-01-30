use anyhow::Result;
use orinium_browser::browser::BrowserApp;
use std::env;
use orinium_browser::platform::audio::SoundManager;

fn main() -> Result<()> {
    let audio_file = "resource:///audio/birds.mp3";

    let args: Vec<String> = env::args().collect();
    let _font_path = if args.len() > 1 {
        Some(args[1].clone())
    } else {
        None
    };

    env_logger::init();

    let browser = BrowserApp::default();

    let mgr = SoundManager::init().expect("Failed to initialize SoundManager");
    {
        let mut sm = match mgr.lock() {
            Ok(x) => x,
            Err(_) => panic!("Failed to lock SoundManager"),
        };
        sm.play_from_uri(audio_file).expect("Failed to play audio");
    }

    browser.run()?;

    Ok(())
}
