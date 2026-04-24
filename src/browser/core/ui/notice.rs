//! 致命的なエラーが発生した際（システムフォントが存在しないなど）に使用するメッセージボックスを表示するユーティリティー

use rfd::MessageDialog;

pub fn show_error_window(title: String, msg: String) {
    MessageDialog::new()
        .set_title(title)
        .set_description(msg)
        .set_buttons(rfd::MessageButtons::Ok)
        .set_level(rfd::MessageLevel::Error)
        .show();
}