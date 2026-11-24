//! Write one pasteboard item so spreadsheets can select TSV and editors can select plain text.
use gpui_kit::{App, ClipboardItem, Global};

/// All document windows write to one clipboard. Only the latest request may publish its result.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct CopyRequest(u64);

impl Global for CopyRequest {}

impl CopyRequest {
    pub fn begin(cx: &mut App) -> Self {
        let latest = cx.default_global::<Self>();
        latest.0 = latest.0.wrapping_add(1);
        *latest
    }

    pub fn is_current(self, cx: &App) -> bool {
        cx.try_global::<Self>() == Some(&self)
    }
}

pub(crate) fn write_text(value: String, cx: &mut App) {
    CopyRequest::begin(cx);
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
}
