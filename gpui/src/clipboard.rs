//! Write one pasteboard item so spreadsheets can select TSV and editors can select plain text.
use gpui_kit::{App, ClipboardItem, Global};

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
            Self::from_board(&objc2_app_kit::NSPasteboard::generalPasteboard())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self::Untracked
        }
    }

    #[cfg(target_os = "macos")]
    fn from_board(board: &objc2_app_kit::NSPasteboard) -> Self {
        Self::ChangeCount(board.changeCount())
    }
}

pub(crate) fn write_text(value: String, cx: &mut App) {
    CopyGeneration::advance(cx);
    cx.write_to_clipboard(ClipboardItem::new_string(value));
}

pub(crate) fn write_tsv(value: String, cx: &mut App) -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    if !cx.is_test() {
        return macos::write(&objc2_app_kit::NSPasteboard::generalPasteboard(), &value);
    }
    cx.write_to_clipboard(ClipboardItem::new_string(value));
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSString};

    const TYPES: [&str; 2] = [
        "public.utf8-plain-text",
        "public.utf8-tab-separated-values-text",
    ];

    pub(super) fn write(board: &NSPasteboard, value: &str) -> Result<(), &'static str> {
        let item = NSPasteboardItem::new();
        let text = NSString::from_str(value);
        for kind in TYPES {
            if !item.setString_forType(&text, &NSString::from_str(kind)) {
                return Err("Could not prepare the clipboard contents");
            }
        }
        let writer: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*item);
        board.clearContents();
        board
            .writeObjects(&NSArray::from_slice(&[writer]))
            .then_some(())
            .ok_or("Could not write to the clipboard")
    }

    #[test]
    fn macos_reads_both_formats_from_a_single_pasteboard_item() {
        let board = NSPasteboard::pasteboardWithUniqueName();
        let text = "café\t温度\r\n";
        write(&board, text).unwrap();
        let items = board.pasteboardItems().unwrap();
        assert_eq!(items.count(), 1);
        let item = items.objectAtIndex(0);
        for kind in TYPES {
            assert_eq!(
                item.stringForType(&NSString::from_str(kind))
                    .unwrap()
                    .to_string(),
                text
            );
        }
        board.clearContents();
    }

    #[test]
    fn copying_identical_contents_advances_the_native_pasteboard() {
        let board = NSPasteboard::pasteboardWithUniqueName();
        write(&board, "same").unwrap();
        let pending = super::ClipboardState::from_board(&board);
        assert!(pending == super::ClipboardState::from_board(&board));
        write(&board, "same").unwrap();
        assert!(pending != super::ClipboardState::from_board(&board));
        board.clearContents();
    }
}
