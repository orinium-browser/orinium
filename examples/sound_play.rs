use anyhow::Result;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let filename = "audios/test1.mp3";
    let bytes = orinium_browser::platform::io::load_resource(filename).await?;
    let handle = orinium_browser::platform::audio::play_bytes(&bytes)?;

    println!("Playing resource:///{}...", filename);

    // 自動デモ: 2秒再生 -> pause -> 2秒 -> resume at 20% volume -> 3秒 -> full volume -> 2秒 -> stop
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("Pausing...");
    handle.pause();

    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("Resuming and set volume to 0.2...");
    handle.play();
    handle.set_volume(0.2);

    tokio::time::sleep(Duration::from_secs(3)).await;
    println!("Set volume to 1.0...");
    handle.set_volume(1.0);

    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("Stopping...");
    handle.stop();

    println!("Playback demo finished.");

    Ok(())
}

// このexampleに使用されている音源（resource://audios/test1.mp3)はOceanKing様によって作成された以下の音源を使用しています。
// 作曲された方に感謝します。
// この音源はhttps://pixabay.com/ja/service/license-summary/に基づき、商用・非商用問わず無料で使用可能です。
// https://pixabay.com/ja/music/%e3%83%a1%e3%82%a4%e3%83%b3%e3%82%bf%e3%82%a4%e3%83%88%e3%83%ab-roaming-the-desert-with-brendan-fraser-219729/