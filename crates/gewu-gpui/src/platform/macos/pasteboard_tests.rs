use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSString;

use super::{TYPES, write};

#[test]
fn both_formats_are_read_from_a_single_pasteboard_item() {
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
fn copying_identical_contents_advances_the_change_count() {
    let board = NSPasteboard::pasteboardWithUniqueName();
    write(&board, "same").unwrap();
    let pending = board.changeCount();
    assert_eq!(pending, board.changeCount());
    write(&board, "same").unwrap();
    assert_ne!(pending, board.changeCount());
    board.clearContents();
}
