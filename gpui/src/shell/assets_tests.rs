use gpui_kit::assets::IconName;

use super::*;

#[test]
fn application_source_loads_the_filter_locator_and_default_icons() {
    for icon in [IconName::Locate, IconName::Search] {
        let bytes = AppAssets.load(&icon.path()).unwrap().unwrap();
        assert!(bytes.starts_with(b"<svg"));
    }
}
