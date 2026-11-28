//! The file picker: the native open panel on macOS, otherwise the toolkit's path prompt.
use std::path::PathBuf;

use gpui_kit::*;

use super::{Documents, open_with_header, report_window_error};

/// Why the picker could not offer files.
pub(crate) enum PickerError {
    NotMainThread,
    /// AppKit did not create an open panel.
    Unavailable,
    Prompt(Box<dyn std::error::Error + Send + Sync>),
}

pub(super) struct FilePicker {
    #[cfg(target_os = "macos")]
    panel: Option<crate::platform::macos::panels::OpenPanel>,
    _selection: Task<()>,
}

impl FilePicker {
    pub fn show(cx: &mut App) -> Result<Self, PickerError> {
        #[cfg(target_os = "macos")]
        if !cx.is_test() {
            let main = objc2::MainThreadMarker::new().ok_or(PickerError::NotMainThread)?;
            let (panel, chosen) =
                crate::platform::macos::panels::open_files(main, super::supports_header)
                    .ok_or(PickerError::Unavailable)?;
            let selection = async move {
                Ok(chosen
                    .recv()
                    .await
                    .ok()
                    .flatten()
                    .map(|files| (files.paths, files.has_header)))
            };
            return Ok(Self {
                panel: Some(panel),
                _selection: Self::receive(selection, cx),
            });
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open".into()),
        });
        let selection = async move {
            paths
                .await
                .unwrap_or(Ok(None))
                .map(|paths| paths.map(|p| (p, true)))
                .map_err(|error| PickerError::Prompt(error.into()))
        };
        Ok(Self {
            #[cfg(target_os = "macos")]
            panel: None,
            _selection: Self::receive(selection, cx),
        })
    }

    fn receive(
        selection: impl Future<Output = Result<Option<(Vec<PathBuf>, bool)>, PickerError>> + 'static,
        cx: &mut App,
    ) -> Task<()> {
        cx.spawn(async move |cx| {
            let result = selection.await;
            cx.update(|cx| {
                cx.global_mut::<Documents>().picker = None;
                match result {
                    Ok(Some((paths, header))) => open_with_header(paths, header, cx),
                    Err(error) => {
                        report_window_error(&crate::ui::errors::picker_message(&error), cx)
                    }
                    Ok(None) => {}
                }
            });
        })
    }

    pub fn activate(&self) {
        #[cfg(target_os = "macos")]
        if let Some(panel) = &self.panel {
            panel.activate();
        }
    }

    pub fn dismiss(&self) {
        #[cfg(target_os = "macos")]
        if let Some(panel) = &self.panel {
            panel.dismiss();
        }
    }
}
