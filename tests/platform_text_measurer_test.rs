use orinium_browser::engine::bridge::text::{
    FontDescription, LayoutConstraints, TextMeasurementRequest, TextMeasurer,
};
use orinium_browser::platform::renderer::text_measurer::PlatformTextMeasurer;

#[test]
fn platform_text_measurer_from_bytes_smoke() {
    let candidates = [
        "C:\\Windows\\Fonts\\arial.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
    ];

    let mut font_path = None;
    for p in candidates.iter() {
        if std::path::Path::new(p).exists() {
            font_path = Some(p.to_string());
            break;
        }
    }

    let path = match font_path {
        Some(p) => p,
        None => {
            eprintln!("skipping PlatformTextMeasurer test: no system font found");
            return;
        }
    };

    let bytes = std::fs::read(path).expect("read font");
    let pm = PlatformTextMeasurer::from_bytes("t", bytes).expect("create measurer");

    let req = TextMeasurementRequest {
        text: "Hello, world!".to_string(),
        font: FontDescription {
            family: None,
            size_px: 16.0,
        },
        constraints: LayoutConstraints {
            max_width: Some(200.0),
            wrap: true,
            max_lines: None,
        },
    };

    let res = pm.measure(&req).expect("measure");
    println!("measured w={} h={}", res.width, res.height);
    assert!(res.width > 0.0);
    assert!(res.height > 0.0);
}
