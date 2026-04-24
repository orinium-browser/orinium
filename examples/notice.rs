use orinium_browser::browser::core::ui::notice::show_error_window;

fn main() {
    show_error_window("致命的なエラー".to_string(), "致命的なエラーが発生したときに表示するウィンドウテスト".to_string());
}