//! The export save panel: the system format menu offers each export format, the file name's
//! extension follows the chosen format, and Parquet adds its compression choice below.
use std::path::{Path, PathBuf};

use block2::RcBlock;
use gewu_core::{FileType, ParquetCompression};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{
    NSModalResponseOK, NSOpenSavePanelDelegate, NSPopUpButton, NSSavePanel, NSTextField, NSView,
    NSWindow,
};
use objc2_foundation::{
    NSArray, NSEdgeInsets, NSObject, NSObjectProtocol, NSRect, NSString, NSURL,
};
use objc2_uniform_type_identifiers::UTType;

use super::{Choice, accessory, choice, content_type};

/// What the panel offers and starts with. Labels are the interface's names for each choice.
pub(crate) struct ExportPanelOptions<'a> {
    pub title: &'a str,
    pub message: &'a str,
    pub directory: &'a Path,
    /// The file name without its extension; the chosen format's extension is appended.
    pub stem: &'a str,
    pub formats: &'a [(FileType, String)],
    pub format: FileType,
    pub compression_label: &'a str,
    pub compressions: &'a [(ParquetCompression, &'static str)],
    pub compression: ParquetCompression,
}

/// What the person chose in a completed panel.
pub(crate) struct ExportChoice {
    pub path: PathBuf,
    pub format: FileType,
    pub compression: ParquetCompression,
}

struct Selection {
    formats: Vec<(FileType, Retained<UTType>, Retained<NSString>)>,
    compression: Retained<NSPopUpButton>,
    /// Shown below the panel only while Parquet is selected.
    accessory: Retained<NSView>,
}

impl Selection {
    fn format(&self, kind: Option<&UTType>) -> Option<FileType> {
        let kind = kind?;
        self.formats
            .iter()
            .find(|(_, uttype, _)| uttype.identifier() == kind.identifier())
            .map(|(format, ..)| *format)
    }

    fn show_options(&self, panel: &NSSavePanel, kind: Option<&UTType>) {
        let parquet = self.format(kind) == Some(FileType::Parquet);
        panel.setAccessoryView(parquet.then_some(&*self.accessory));
    }
}

#[expect(
    unsafe_code,
    reason = "AppKit asks the panel's delegate for format names and reports format changes"
)]
mod delegate {
    use super::*;

    define_class!(
        // SAFETY: NSObject places no additional requirements on subclasses.
        #[unsafe(super = NSObject)]
        #[name = "GewuExportPanelDelegate"]
        #[thread_kind = MainThreadOnly]
        #[ivars = Selection]
        pub(super) struct Delegate;

        // SAFETY: This class inherits the NSObject protocol implementation.
        unsafe impl NSObjectProtocol for Delegate {}
        // SAFETY: Both selectors have the NSOpenSavePanelDelegate signatures.
        unsafe impl NSOpenSavePanelDelegate for Delegate {
            #[unsafe(method_id(panel:displayNameForType:))]
            fn display_name(
                &self,
                _sender: &AnyObject,
                kind: &UTType,
            ) -> Option<Retained<NSString>> {
                self.ivars()
                    .formats
                    .iter()
                    .find(|(_, uttype, _)| uttype.identifier() == kind.identifier())
                    .map(|(.., label)| label.clone())
            }

            #[unsafe(method(panel:didSelectType:))]
            fn selected(&self, sender: &AnyObject, kind: Option<&UTType>) {
                if let Some(panel) = sender.downcast_ref::<NSSavePanel>() {
                    self.ivars().show_options(panel, kind);
                }
            }
        }
    );

    pub(super) fn new(main: MainThreadMarker, selection: Selection) -> Retained<Delegate> {
        let this = Delegate::alloc(main).set_ivars(selection);
        // SAFETY: NSObject's initializer is called once on the allocated object with initialized
        // ivars.
        unsafe { msg_send![super(this), init] }
    }
}

fn compression_options(
    main: MainThreadMarker,
    options: &ExportPanelOptions,
) -> (Retained<NSPopUpButton>, Retained<NSView>) {
    let label = NSTextField::labelWithString(&NSString::from_str(options.compression_label), main);
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(main),
        NSRect::default(),
        false,
    );
    for (_, title) in options.compressions {
        popup.addItemWithTitle(&NSString::from_str(title));
    }
    if let Some(index) = options
        .compressions
        .iter()
        .position(|(compression, _)| *compression == options.compression)
    {
        popup.selectItemAtIndex(index as isize);
    }
    let stack = accessory(main, &[&label, &popup]);
    stack.setEdgeInsets(NSEdgeInsets {
        top: 12.,
        left: 16.,
        bottom: 12.,
        right: 16.,
    });
    stack.setFrameSize(stack.fittingSize());
    (popup, Retained::into_super(stack))
}

/// Show the export panel, as a sheet on `window` when given. `None` when the panel could not
/// be shown.
#[expect(
    unsafe_code,
    reason = "AppKit's panel constructor and weak delegate are checked here"
)]
pub(crate) fn choose_export(
    main: MainThreadMarker,
    window: Option<&NSWindow>,
    options: &ExportPanelOptions,
) -> Option<Choice<ExportChoice>> {
    // SAFETY: This class selector is called on the main thread. AppKit can return nil despite
    // objc2-app-kit's non-null 0.3 binding. Capture the autoreleased result as an Option.
    let panel: Option<Retained<NSSavePanel>> = unsafe { msg_send![class!(NSSavePanel), savePanel] };
    let panel = panel?;
    let formats: Vec<_> = options
        .formats
        .iter()
        .map(|(format, label)| (*format, content_type(*format), NSString::from_str(label)))
        .collect();
    let types: Vec<&UTType> = formats.iter().map(|(_, uttype, _)| &**uttype).collect();
    let initial = formats
        .iter()
        .find(|(format, ..)| *format == options.format)
        .map(|(_, uttype, _)| uttype.clone());
    let (compression, accessory) = compression_options(main, options);
    let compressions: Vec<_> = options.compressions.iter().map(|(c, _)| *c).collect();
    panel.setTitle(Some(&NSString::from_str(options.title)));
    panel.setPrompt(Some(&NSString::from_str(options.title)));
    panel.setMessage(Some(&NSString::from_str(options.message)));
    panel.setDirectoryURL(NSURL::from_directory_path(options.directory).as_deref());
    panel.setNameFieldStringValue(&NSString::from_str(&format!(
        "{}.{}",
        options.stem,
        options.format.extension()
    )));
    panel.setAllowedContentTypes(&NSArray::from_slice(&types));
    panel.setCurrentContentType(initial.as_deref());
    // An accessory view hides the format menu unless content types are shown explicitly.
    panel.setShowsContentTypes(true);
    panel.setCanCreateDirectories(true);
    panel.setExtensionHidden(false);
    panel.setShowsTagField(false);
    let (on_finish, chosen) = choice();
    let delegate = delegate::new(main, Selection {
        formats,
        compression,
        accessory,
    });
    delegate
        .ivars()
        .show_options(&panel, panel.currentContentType().as_deref());
    // SAFETY: The panel holds a weak reference. Its completion block retains the delegate
    // until AppKit has finished sending format changes.
    unsafe { panel.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    let completion = RcBlock::new({
        let panel = panel.clone();
        move |response| {
            // SAFETY: Modal interaction has ended; detach the weak delegate before releasing it.
            unsafe { panel.setDelegate(None) };
            let selection = delegate.ivars();
            let choice = (response == NSModalResponseOK)
                .then(|| {
                    let path = panel.URL()?.to_file_path()?;
                    let format = selection
                        .format(panel.currentContentType().as_deref())
                        .or_else(|| FileType::from_path(&path))?;
                    let index = selection.compression.indexOfSelectedItem();
                    let compression = usize::try_from(index)
                        .ok()
                        .and_then(|index| compressions.get(index).copied())
                        .unwrap_or_default();
                    Some(ExportChoice {
                        path,
                        format,
                        compression,
                    })
                })
                .flatten();
            on_finish(choice);
        }
    });
    match window {
        Some(window) => panel.beginSheetModalForWindow_completionHandler(window, &completion),
        None => panel.beginWithCompletionHandler(&completion),
    }
    Some(chosen)
}
