//! Icons bundled with the app beyond the component library's set.
use std::borrow::Cow;

use gpui_kit::{
    AssetSource, Result, SharedString,
    assets::{Assets, icon_assets},
};

icon_assets!(ExtraIcons, [Locate]);

pub(crate) struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = ExtraIcons.load(path)? {
            return Ok(Some(icon));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut icons = Assets.list(path)?;
        icons.extend(ExtraIcons.list(path)?);
        Ok(icons)
    }
}

#[cfg(test)]
#[path = "assets_tests.rs"]
mod tests;
