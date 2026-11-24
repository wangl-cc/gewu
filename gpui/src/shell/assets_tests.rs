use gpui_kit::assets::IconName;

use super::*;

#[test]
fn application_source_loads_the_filter_locator_warning_and_search_icons() {
    for icon in [IconName::Locate, IconName::TriangleAlert, IconName::Search] {
        let bytes = AppAssets.load(&icon.path()).unwrap().unwrap();
        assert!(bytes.starts_with(b"<svg"));
    }
}
