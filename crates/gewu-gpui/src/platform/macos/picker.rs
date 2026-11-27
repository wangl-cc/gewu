//! The document picker owns the first-row option while the panel is open.
use std::path::{Path, PathBuf};

use block2::RcBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, class, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{
    NSButton, NSControlStateValueOn, NSModalResponseOK, NSOpenPanel, NSOpenSavePanelDelegate,
};
use objc2_foundation::{NSObject, NSObjectProtocol, ns_string};

struct Options {
    header: Retained<NSButton>,
    supports_header: fn(&Path) -> bool,
}

#[expect(
    unsafe_code,
    reason = "AppKit delivers the panel's selection through this Objective-C protocol"
)]
mod delegate {
    use super::*;

    define_class!(
        // SAFETY: NSObject places no additional requirements on subclasses.
        #[unsafe(super = NSObject)]
        #[name = "GewuOpenPanelDelegate"]
        #[thread_kind = MainThreadOnly]
        #[ivars = Options]
        pub(super) struct Delegate;

        // SAFETY: This class inherits the NSObject protocol implementation.
        unsafe impl NSObjectProtocol for Delegate {}
        // SAFETY: The one implemented selector has the NSOpenSavePanelDelegate signature.
        unsafe impl NSOpenSavePanelDelegate for Delegate {
            #[unsafe(method(panelSelectionDidChange:))]
            fn selection_changed(&self, sender: Option<&AnyObject>) {
                let Some(panel) = sender.and_then(|s| s.downcast_ref::<NSOpenPanel>()) else {
                    return;
                };
                let options = self.ivars();
                let eligible = panel.URLs().iter().any(|url| {
                    !url.hasDirectoryPath()
                        && url
                            .to_file_path()
                            .is_some_and(|p| (options.supports_header)(&p))
                });
                if eligible != panel.accessoryView().is_some() {
                    panel.setAccessoryView(eligible.then_some(&options.header));
                    if eligible {
                        panel.setAccessoryViewDisclosed(true);
                    }
                }
            }
        }
    );

    pub(super) fn new(main: MainThreadMarker, options: Options) -> Retained<Delegate> {
        let this = Delegate::alloc(main).set_ivars(options);
        // SAFETY: NSObject's initializer is called once on the allocated object with initialized
        // ivars.
        unsafe { msg_send![super(this), init] }
    }
}

/// An open panel shown by [`open_files`].
pub(crate) struct OpenPanel(Retained<NSOpenPanel>);

impl OpenPanel {
    /// Bring the panel back to the front.
    pub(crate) fn activate(&self) {
        self.0.makeKeyAndOrderFront(None);
    }

    /// Close the panel as if cancelled.
    pub(crate) fn dismiss(&self) {
        self.0.performClose(None);
    }
}

/// Show a multi-file panel. The header choice applies to paths accepted by `supports_header`.
#[expect(
    unsafe_code,
    reason = "AppKit's panel constructor, action and weak delegate are checked here"
)]
pub(crate) fn open_files(
    main: MainThreadMarker,
    supports_header: fn(&Path) -> bool,
    on_finish: impl Fn(Option<(Vec<PathBuf>, bool)>) + 'static,
) -> Option<OpenPanel> {
    // SAFETY: This class selector is called on the main thread. AppKit can return nil despite
    // objc2-app-kit's non-null 0.3 binding. Capture the autoreleased result as an Option.
    let panel: Option<Retained<NSOpenPanel>> = unsafe { msg_send![class!(NSOpenPanel), openPanel] };
    let panel = panel?;
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(true);
    panel.setResolvesAliases(true);
    // SAFETY: With no target and no action, this standard checkbox keeps its own checked state.
    let header = unsafe {
        NSButton::checkboxWithTitle_target_action(
            ns_string!("Use first row as column names (CSV, TSV, Excel)"),
            None,
            None,
            main,
        )
    };
    header.setState(NSControlStateValueOn);
    header.sizeToFit();
    let delegate = delegate::new(main, Options {
        header,
        supports_header,
    });
    // SAFETY: The panel holds a weak reference. Its completion block retains the delegate
    // until AppKit has finished sending selection changes.
    unsafe { panel.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    let completion = RcBlock::new({
        let panel = panel.clone();
        move |response| {
            // SAFETY: Modal interaction has ended; detach the weak delegate before releasing it.
            unsafe { panel.setDelegate(None) };
            on_finish((response == NSModalResponseOK).then(|| {
                let paths = panel
                    .URLs()
                    .iter()
                    .filter_map(|url| url.to_file_path())
                    .collect();
                (
                    paths,
                    delegate.ivars().header.state() == NSControlStateValueOn,
                )
            }));
        }
    });
    panel.beginWithCompletionHandler(&completion);
    Some(OpenPanel(panel))
}
