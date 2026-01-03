use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use dasp_sample::FromSample;

/// 音声再生ハンドル。これを保持している間、再生が継続します。
pub struct PlayHandle {
    // Keep stream and sink alive
    _stream: rodio::OutputStream,
    sink: Arc<rodio::Sink>,
}

impl PlayHandle {
    /// 再生を停止し、ハンドルを破棄する。
    pub fn stop(self) {
        // sink.stop() を呼んでからハンドルを破棄する
        self.sink.stop();
        // _stream と sink はここで drop される
    }

    /// 再生を一時停止する。
    pub fn pause(&self) {
        self.sink.pause();
    }

    /// 一時停止から再開する。
    pub fn play(&self) {
        self.sink.play();
    }

    /// 音量を設定する（0.0 = ミュート、1.0 = デフォルト）。
    pub fn set_volume(&self, value: f32) {
        self.sink.set_volume(value);
    }

    /// 現在一時停止状態かを返す。
    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    /// 再生中のSinkを取得（必要なら）.
    pub fn sink(&self) -> Arc<rodio::Sink> {
        Arc::clone(&self.sink)
    }
}

/// ファイルパスから音声を再生し、再生ハンドルを返す。
/// サポートされるフォーマットはrodioに依存します（wav/mp3/ogg等）。
pub fn play_file<P: AsRef<Path>>(path: P) -> anyhow::Result<PlayHandle> {
    let path = path.as_ref();
    let file = std::fs::File::open(path).with_context(|| format!("failed to open file: {}", path.display()))?;
    let decoder = rodio::Decoder::new(std::io::BufReader::new(file))?;

    play_source(decoder)
}

/// バイトスライスから音声を再生し、再生ハンドルを返す。
pub fn play_bytes(bytes: &[u8]) -> anyhow::Result<PlayHandle> {
    let cursor = std::io::Cursor::new(bytes.to_vec());
    let decoder = rodio::Decoder::new(std::io::BufReader::new(cursor))?;
    play_source(decoder)
}

fn play_source<S>(source: S) -> anyhow::Result<PlayHandle>
where
    S: rodio::Source + Send + 'static,
    f32: FromSample<S::Item>,
{
    // Open default output stream
    let stream = rodio::OutputStreamBuilder::open_default_stream().context("failed to open output stream")?;
    let mixer = stream.mixer();
    let sink = rodio::Sink::connect_new(mixer);
    sink.append(source);

    Ok(PlayHandle { _stream: stream, sink: Arc::new(sink) })
}

/// 簡易: fire-and-forget 再生。内部でハンドルを破棄せずスレッド上で保持する。
pub fn play_file_fire_and_forget<P: AsRef<Path> + Send + 'static>(path: P) -> anyhow::Result<()> {
    let path = path.as_ref().to_owned();
    std::thread::spawn(move || {
        if let Ok(handle) = play_file(path) {
            // wait until finished
            let s = handle.sink();
            s.sleep_until_end();
        }
    });
    Ok(())
}
