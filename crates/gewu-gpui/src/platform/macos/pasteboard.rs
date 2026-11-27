//! The general pasteboard: writing one item with plain and tab-separated text, and the change
//! count that tells whether anything else wrote since.
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSString};

const TYPES: [&str; 2] = [
    "public.utf8-plain-text",
    "public.utf8-tab-separated-values-text",
];

/// Why the pasteboard did not take the copied text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasteboardError {
    Prepare,
    Write,
}

/// Write `value` to the general pasteboard as one item spreadsheets read as TSV and editors
/// as plain text.
pub(crate) fn write_tsv(value: &str) -> Result<(), PasteboardError> {
    write(&NSPasteboard::generalPasteboard(), value)
}

/// The general pasteboard's change count, which advances on every write by any application.
pub(crate) fn change_count() -> isize {
    NSPasteboard::generalPasteboard().changeCount()
}

fn write(board: &NSPasteboard, value: &str) -> Result<(), PasteboardError> {
    let item = NSPasteboardItem::new();
    let text = NSString::from_str(value);
    for kind in TYPES {
        if !item.setString_forType(&text, &NSString::from_str(kind)) {
            return Err(PasteboardError::Prepare);
        }
    }
    let writer: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*item);
    board.clearContents();
    board
        .writeObjects(&NSArray::from_slice(&[writer]))
        .then_some(())
        .ok_or(PasteboardError::Write)
}

#[cfg(test)]
#[path = "pasteboard_tests.rs"]
mod tests;
