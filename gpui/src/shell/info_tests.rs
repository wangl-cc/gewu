use gewu_core::{DatasetInfo, FileMetadata, FileType};
use gpui_kit::{TestAppContext, VisualTestContext, px, test::TestWindowExt};

use super::{format_file_size, open};

#[test]
fn metadata_sizes_keep_exact_bytes_and_use_decimal_units() {
    assert_eq!(format_file_size(0), "0 bytes");
    assert_eq!(format_file_size(1), "1 byte");
    assert_eq!(format_file_size(999), "999 bytes");
    assert_eq!(format_file_size(6_186), "6.2 KB (6,186 bytes)");
    assert_eq!(format_file_size(24_000_321), "24.0 MB (24,000,321 bytes)");
}

#[test]
fn copy_path_uses_the_complete_location_in_a_compact_info_window() {
    let app = TestAppContext::single();
    let path = format!("/{}report.csv", "long-directory/".repeat(12));
    let window = app.update(|cx| {
        gpui_kit::init(cx);
        open(
            DatasetInfo {
                path: path.clone(),
                row_count: 12,
                columns: vec![],
                file_size: 6_186,
                format: FileType::Csv,
                metadata: FileMetadata::Delimited,
            },
            cx,
        );
        cx.windows()[0]
    });
    let mut ui = VisualTestContext::from_window(window, &app);
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("info-location").label(), Some(path.as_str()));
        let reveal = w.find("reveal-file").bounds();
        assert!(reveal.size.width < w.viewport_size().width / 2.);
        assert!(reveal.bottom() <= w.viewport_size().height);
        assert!(w.viewport_size().height - reveal.bottom() < px(60.));
        w.click("copy-path", cx);
    });
    assert_eq!(
        app.read_from_clipboard().unwrap().text().as_deref(),
        Some(path.as_str())
    );
    app.quit();
}
