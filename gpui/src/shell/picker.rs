use std::path::PathBuf;

use gpui_kit::*;
#[cfg(target_os = "macos")]
use {objc2::rc::Retained, objc2_app_kit::NSOpenPanel};

use super::{Documents, open_with_header, report_window_error};

pub(super) struct FilePicker {
    #[cfg(target_os = "macos")]
    panel: Option<Retained<NSOpenPanel>>,
    _selection: Task<()>,
}

impl FilePicker {
    pub fn show(cx: &mut App) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        if !cx.is_test() {
            let main =
                objc2::MainThreadMarker::new().ok_or("The file picker needs the main thread.")?;
            let (sender, receiver) = async_channel::bounded(1);
            let panel = gewu_macos::open_files(main, super::supports_header, move |choice| {
                let _ = sender.try_send(choice);
            })
            .ok_or("macOS could not open the file picker.")?;
            return Ok(Self {
                panel: Some(panel),
                _selection: Self::receive(
                    async move { Ok(receiver.recv().await.ok().flatten()) },
                    cx,
                ),
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
                .map_err(|e| e.to_string())
        };
        Ok(Self {
            #[cfg(target_os = "macos")]
            panel: None,
            _selection: Self::receive(selection, cx),
        })
    }

    fn receive(
        selection: impl Future<Output = Result<Option<(Vec<PathBuf>, bool)>, String>> + 'static,
        cx: &mut App,
    ) -> Task<()> {
        cx.spawn(async move |cx| {
            let result = selection.await;
            cx.update(|cx| {
                cx.global_mut::<Documents>().picker = None;
                match result {
                    Ok(Some((paths, header))) => open_with_header(paths, header, cx),
                    Err(error) => report_window_error(error, cx),
                    Ok(None) => {}
                }
            });
        })
    }

    pub fn activate(&self) {
        #[cfg(target_os = "macos")]
        if let Some(panel) = &self.panel {
            panel.makeKeyAndOrderFront(None);
        }
    }

    pub fn dismiss(&self) {
        #[cfg(target_os = "macos")]
        if let Some(panel) = &self.panel {
            panel.performClose(None);
        }
    }
}
