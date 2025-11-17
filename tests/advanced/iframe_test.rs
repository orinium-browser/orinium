//! viewportにviewportを埋め込むiframeのテスト
//! iframe内のスクロールバーの動作確認

mod utils;


#[test]
fn test_iframe_rendering() {
    let html = utils::TEST_HTML;
    let renderer = utils::renderer_maker(html, "");
}