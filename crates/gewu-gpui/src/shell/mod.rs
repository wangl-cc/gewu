//! Files are opened in separate document windows. Each window owns one replaceable load.
use std::path::{Path, PathBuf};

use document::Document;
use gpui_kit::{base::input, *};

use crate::{actions::*, platform};

pub(crate) mod assets;
pub(crate) mod document;
mod export;
pub(crate) use export::ExportFailure;
mod import;
mod info;
mod picker;
pub(crate) use picker::PickerError;
mod update;

gpui_kit::actions!(gewu, [
    OpenFiles,
    ClearRecent,
    GetInfo,
    ImportOptions,
    CloseWindow,
    MinimizeWindow,
    ZoomWindow,
    ToggleFullScreen,
    HideGewu,
    HideOthers,
    ShowAllApps,
    Preferences,
    Quit
]);
#[derive(Action, Clone, Debug, Default, PartialEq)]
#[action(namespace = gewu, no_json)]
struct OpenRecent(PathBuf);

struct Documents {
    windows: Vec<(PathBuf, AnyWindowHandle)>,
    recent: platform::RecentDocuments,
    picker: Option<picker::FilePicker>,
    _events: Task<()>,
}
impl Global for Documents {}

pub(crate) fn run(arguments: impl Iterator<Item = PathBuf>) {
    let app = gpui_kit::application().with_assets(assets::AppAssets);
    let (send, receive) = async_channel::unbounded();
    app.on_open_urls(move |urls| {
        let paths: Vec<_> = urls
            .into_iter()
            .filter_map(|s| url::Url::parse(&s).ok()?.to_file_path().ok())
            .collect();
        if !paths.is_empty() {
            // The receiver only closes when the application is quitting.
            let _ = send.try_send(paths);
        }
    });
    app.on_reopen(|cx| {
        if cx
            .try_global::<Documents>()
            .is_some_and(|docs| !docs.has_open_window(cx))
        {
            choose_files(cx);
        }
    });
    let mut initial: Vec<_> = arguments
        .filter(|path| {
            path != Path::new("--") && !path.as_os_str().to_string_lossy().starts_with("-psn_")
        })
        .collect();
    app.run(move |cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        while let Ok(paths) = receive.try_recv() {
            initial.extend(paths);
        }
        crate::preferences::init(cx);
        #[cfg(target_os = "macos")]
        platform::observe_display_options(cx);
        install_file_actions(platform::RecentDocuments::system(), receive, cx);
        cx.activate(true);
        if initial.is_empty() {
            cx.defer(|cx| {
                if !cx.global::<Documents>().has_open_window(cx) {
                    choose_files(cx)
                }
            });
        } else {
            open_paths(initial, cx);
        }
    });
}

impl Documents {
    fn has_open_window(&self, cx: &App) -> bool {
        let windows = cx.windows();
        self.windows
            .iter()
            .any(|(_, handle)| windows.contains(handle))
    }
}

pub(crate) fn install_file_actions(
    recent: platform::RecentDocuments,
    receive: async_channel::Receiver<Vec<PathBuf>>,
    cx: &mut App,
) {
    let events = cx.spawn(async move |cx| {
        while let Ok(paths) = receive.recv().await {
            cx.update(|cx| open_paths(paths, cx));
        }
    });
    cx.set_global(Documents {
        windows: Vec::new(),
        recent,
        picker: None,
        _events: events,
    });
    update::init(cx);
    cx.on_action(|_: &OpenFiles, cx| choose_files(cx));
    cx.on_action(|_: &ClearRecent, cx| {
        cx.global_mut::<Documents>().recent.clear();
        update_menus(cx);
    });
    cx.on_action(|_: &Preferences, cx| crate::preferences::open(cx));
    cx.on_action(|OpenRecent(path), cx| open_paths(vec![path.clone()], cx));
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &HideGewu, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
    cx.on_action(|_: &MinimizeWindow, cx| with_active_window(cx, Window::minimize_window));
    cx.on_action(|_: &ZoomWindow, cx| with_active_window(cx, Window::zoom_window));
    cx.on_action(|_: &ToggleFullScreen, cx| with_active_window(cx, Window::toggle_fullscreen));
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(active) = cx.active_window() {
            let _ = active.update(cx, |_, w, _| w.remove_window());
        }
    });
    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenFiles, None),
        KeyBinding::new("cmd-i", GetInfo, None),
        KeyBinding::new("cmd-,", Preferences, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-h", HideGewu, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    cx.observe_global::<crate::preferences::Preferences>(|cx| update_menus(cx))
        .detach();
    update_menus(cx);
}

fn with_active_window(cx: &mut App, action: fn(&Window)) {
    if let Some(active) = cx.active_window() {
        let _ = active.update(cx, |_, window, _| action(window));
    }
}

pub(crate) fn update_menus(cx: &App) {
    let recent = cx.global::<Documents>().recent.paths();
    let history = Menu::new("Open Recent").disabled(recent.is_empty()).items(
        recent
            .iter()
            .map(|path| MenuItem::action(path.display().to_string(), OpenRecent(path.clone())))
            .chain([
                MenuItem::separator(),
                MenuItem::action("Clear Menu", ClearRecent),
            ]),
    );
    cx.set_menus([
        Menu::new("Gewu").items([
            MenuItem::action("Settings…", Preferences),
            MenuItem::submenu(update::menu(cx)),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Gewu", HideGewu),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAllApps),
            MenuItem::separator(),
            MenuItem::action("Quit Gewu", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Open…", OpenFiles),
            MenuItem::submenu(history),
            MenuItem::action("Import Options…", ImportOptions),
            MenuItem::action("Get Info…", GetInfo),
            MenuItem::action("Export…", ExportTable),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Find…", input::Search),
            MenuItem::action("Find Next", FindNext),
            MenuItem::action("Find Previous", FindPrevious),
        ]),
        Menu::new("View").items([
            MenuItem::action("Columns…", ManageColumns),
            MenuItem::action("Sort…", EditSort),
            MenuItem::separator(),
            MenuItem::action("Toggle Inspector", ShowInspector),
            MenuItem::action("Show Chart Inspector", ShowChartInspector),
            MenuItem::action("Analyze Selected Column", AnalyzeSelectedColumn),
            MenuItem::action("Show Row Numbers", ToggleRowNumbers)
                .checked(crate::preferences::Preferences::get(cx).show_row_numbers),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", ToggleFullScreen),
        ]),
        Menu::new("Filter").items([
            MenuItem::action("Filter Column…", EditFilter),
            MenuItem::action("Filter by Selected Value", FilterSelection),
            MenuItem::separator(),
            MenuItem::action("Clear All Filters", ClearFilters),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
        ]),
    ]);
}

fn report_window_error(message: &str, cx: &mut App) {
    if let Some(active) = cx.active_window() {
        let _ = active.update(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Critical,
                "Could Not Open Table",
                Some(message),
                &["OK"],
                cx,
            )
        });
    } else {
        cx.show_system_notification(SystemNotification {
            tag: "gewu-open-error".into(),
            title: "Could Not Open Table".into(),
            body: message.to_owned().into(),
            actions: Vec::new(),
        });
    }
}

fn choose_files(cx: &mut App) {
    if let Some(picker) = &cx.global::<Documents>().picker {
        picker.activate();
        return;
    }
    match picker::FilePicker::show(cx) {
        Ok(picker) => cx.global_mut::<Documents>().picker = Some(picker),
        Err(error) => report_window_error(&crate::ui::errors::picker_message(&error), cx),
    }
}

pub(crate) fn open_paths(paths: Vec<PathBuf>, cx: &mut App) {
    open_with_header(paths, true, cx);
}

/// Install the app's actions, globals and document handling for a session that tools,
/// examples and tests drive: the given preferences, recent documents kept in memory and no
/// opened-URL events.
pub(crate) fn init_session(preferences: crate::preferences::Preferences, cx: &mut App) {
    gpui_kit::init(cx);
    crate::actions::init(cx);
    cx.set_global(preferences);
    let (_, receive) = async_channel::unbounded();
    install_file_actions(platform::RecentDocuments::session(), receive, cx);
}

/// Open `path` in its own document window, at `window_size` if given.
#[cfg(any(test, feature = "ui-capture"))]
pub(crate) fn open_document_with_size(
    path: PathBuf,
    window_size: Option<Size<Pixels>>,
    cx: &mut App,
) -> Option<(AnyWindowHandle, Entity<Document>)> {
    let mut options = platform::window_options(cx);
    if let Some(size) = window_size {
        options.window_bounds = Some(WindowBounds::Windowed(Bounds::new(Point::default(), size)));
    }
    open_document(canonical(path), true, options, cx).ok()
}

fn canonical(path: PathBuf) -> PathBuf {
    path.canonicalize()
        .or_else(|_| std::path::absolute(&path))
        .unwrap_or(path)
}

/// Open a window for `path`, which must be canonical: windows are found by their path.
fn open_document(
    path: PathBuf,
    has_header: bool,
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<Document>)> {
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Document::new(path.clone(), has_header, window, cx))
    })?;
    cx.global_mut::<Documents>().windows.push((path, opened.0));
    Ok(opened)
}

/// Whether the file at `path` offers reading its first row as column names, judged by name
/// before it is opened.
fn supports_header(path: &Path) -> bool {
    gewu_core::FileType::from_path(path).is_some_and(gewu_core::FileType::has_header_choice)
}

fn open_with_header(paths: Vec<PathBuf>, has_header: bool, cx: &mut App) {
    if !paths.is_empty()
        && let Some(picker) = cx.global_mut::<Documents>().picker.take()
    {
        picker.dismiss();
    }
    let windows = cx.windows();
    cx.global_mut::<Documents>()
        .windows
        .retain(|(_, handle)| windows.contains(handle));
    for path in paths {
        let path = canonical(path);
        if let Some((_, handle)) = cx
            .global::<Documents>()
            .windows
            .iter()
            .find(|(p, _)| p == &path)
        {
            let _ = (*handle).update(cx, |_, w, _| w.activate_window());
            continue;
        }
        // A mixed selection applies the header policy only to eligible files.
        let has_header = has_header || !supports_header(&path);
        let options = platform::window_options(cx);
        if let Err(err) = open_document(path, has_header, options, cx) {
            report_window_error(&err.to_string(), cx);
        }
    }
}

fn record_recent(path: &Path, cx: &mut App) {
    cx.update_global::<Documents, _>(|docs, cx| docs.recent.record(path, cx));
    update_menus(cx);
}
