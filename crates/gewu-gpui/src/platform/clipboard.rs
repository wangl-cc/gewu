//! Copying table text. A copy is current only while no newer copy started, in any window, and
//! nothing else wrote the pasteboard meanwhile: toolkit text fields write it without going
//! through here. Dropping a table's copy task cancels a superseded copy within that table; the
//! generation also orders copies across windows.
use gpui_kit::{App, ClipboardItem, Global};

#[cfg(target_os = "macos")]
pub(crate) use super::macos::pasteboard::PasteboardError;

/// Why the clipboard did not take the copied text.
#[cfg(not(target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasteboardError {}

/// Orders requests across windows, even while their TSV is still being generated.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
struct CopyGeneration(u64);

impl Global for CopyGeneration {}

impl CopyGeneration {
    fn advance(cx: &mut App) -> Self {
        let latest = cx.default_global::<Self>();
        latest.0 = latest.0.wrapping_add(1);
        *latest
    }
}

/// Toolkit text fields write the native pasteboard without going through `write_text`.
pub(crate) struct CopyRequest {
    generation: CopyGeneration,
    clipboard: ClipboardState,
}

impl CopyRequest {
    pub fn begin(cx: &mut App) -> Self {
        Self {
            generation: CopyGeneration::advance(cx),
            clipboard: ClipboardState::read(cx),
        }
    }

    pub fn is_current(&self, cx: &App) -> bool {
        cx.global::<CopyGeneration>() == &self.generation
            && self.clipboard == ClipboardState::read(cx)
    }
}

#[derive(Eq, PartialEq)]
enum ClipboardState {
    #[cfg(target_os = "macos")]
    ChangeCount(isize),
    #[cfg(test)]
    TestContents(Option<ClipboardItem>),
    #[cfg(not(target_os = "macos"))]
    Untracked,
}

impl ClipboardState {
    fn read(_cx: &App) -> Self {
        // The GPUI test platform only exposes the contents of its in-memory clipboard.
        #[cfg(test)]
        if _cx.is_test() {
            return Self::TestContents(_cx.read_from_clipboard());
        }
        #[cfg(target_os = "macos")]
        {
            Self::ChangeCount(super::macos::pasteboard::change_count())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self::Untracked
        }
    }
}

pub(crate) fn write_text(value: String, cx: &mut App) {
    CopyGeneration::advance(cx);
    cx.write_to_clipboard(ClipboardItem::new_string(value));
}

pub(crate) fn write_tsv(value: String, cx: &mut App) -> Result<(), PasteboardError> {
    #[cfg(target_os = "macos")]
    if !cx.is_test() {
        return super::macos::pasteboard::write_tsv(&value);
    }
    cx.write_to_clipboard(ClipboardItem::new_string(value));
    Ok(())
}
