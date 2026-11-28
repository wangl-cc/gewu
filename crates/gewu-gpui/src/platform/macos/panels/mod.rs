//! The system file panels: opening documents and exporting a table. Each panel shows Gewu's
//! options in an accessory view and delivers the choice once, through a channel, when it
//! closes.
use gewu_core::FileType;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSStackView, NSUserInterfaceLayoutOrientation, NSView};
use objc2_foundation::{NSArray, NSString};
use objc2_uniform_type_identifiers::UTType;

mod open;
mod save;
pub(crate) use open::{OpenPanel, open_files};
pub(crate) use save::{ExportPanelOptions, choose_export};

/// What a panel delivers when it closes: the choice, or `None` when cancelled.
pub(crate) type Choice<T> = async_channel::Receiver<Option<T>>;

/// The sending half for a panel's completion handler, and the [`Choice`] its caller awaits.
fn choice<T: 'static>() -> (impl Fn(Option<T>) + 'static, Choice<T>) {
    let (sender, receiver) = async_channel::bounded(1);
    // The caller may have stopped waiting; a closed panel has nothing else to do.
    (move |choice| drop(sender.try_send(choice)), receiver)
}

/// The type the system knows each format by. Parquet is declared by the app bundle.
fn content_type(format: FileType) -> Retained<UTType> {
    let identifier = match format {
        FileType::Parquet => "org.apache.parquet",
        FileType::Csv => "public.comma-separated-values-text",
        FileType::Tsv => "public.tab-separated-values-text",
        FileType::Xlsx => "org.openxmlformats.spreadsheetml.sheet",
    };
    UTType::importedTypeWithIdentifier(&NSString::from_str(identifier))
}

/// An accessory view: `views` side by side, sized to fit them.
fn accessory(main: MainThreadMarker, views: &[&NSView]) -> Retained<NSStackView> {
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(views), main);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    stack.setSpacing(8.);
    stack.setFrameSize(stack.fittingSize());
    stack
}
