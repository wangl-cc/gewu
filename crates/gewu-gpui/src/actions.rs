//! Keyboard commands are scoped to the document, the focused grid or the focused scatter plot.
use gewu_core::{ColumnId, ViewRow};
use gpui_kit::{Action, App, KeyBinding, base::input};

gpui_kit::actions!(gewu, [
    FilterSelection,
    ClearFilters,
    FindNext,
    FindPrevious,
    AnalyzeSelectedColumn,
    ShowChartInspector,
    ToggleRowNumbers,
    ShowInspector,
    EditFilter,
    ManageColumns,
    EditSort,
    ExportTable,
    PreviousPoint,
    NextPoint,
    RevealPoint,
    ClearSorting
]);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Motion {
    #[default]
    Up,
    Down,
    Left,
    Right,
    FirstRow,
    LastRow,
    FirstColumn,
    LastColumn,
    PageUp,
    PageDown,
}

#[derive(Action, Clone, Debug, Default, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct Navigate {
    pub motion: Motion,
    pub extend: bool,
}

/// Context-menu commands on the column or cell the menu was opened on.
#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct SortColumn {
    pub column: ColumnId,
    pub descending: bool,
}

#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct ClearColumnSort {
    pub column: ColumnId,
}

#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct FilterColumn {
    pub column: ColumnId,
}

#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct ClearColumnFilter {
    pub column: ColumnId,
}

#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct AnalyzeColumn {
    pub column: ColumnId,
}

#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct HideColumn {
    pub column: ColumnId,
}

#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct CopyCell {
    pub row: ViewRow,
    pub column: ColumnId,
}

#[derive(Action, Clone, Debug, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct ViewCell {
    pub row: ViewRow,
    pub column: ColumnId,
}

#[derive(Action, Clone, Debug, Default, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct RevealScatterPoint {
    pub chart: gpui_kit::EntityId,
    pub source_row: gewu_core::SourceRow,
}

pub(crate) fn init(cx: &mut App) {
    let table = Some("GewuGrid");
    let document = Some("GewuDocument");
    let scatter = Some("GewuScatter");
    // The component library's debug inspector claims this chord globally, including inside a grid.
    #[cfg(all(debug_assertions, target_os = "macos"))]
    cx.bind_keys([KeyBinding::new(
        "cmd-alt-i",
        gpui_kit::Unbind(gpui_kit::component::ToggleInspector.name().into()),
        None,
    )]);
    cx.bind_keys([
        KeyBinding::new("cmd-a", input::SelectAll, table),
        KeyBinding::new("cmd-c", input::Copy, table),
        KeyBinding::new("cmd-alt-shift-l", FilterSelection, table),
        KeyBinding::new("cmd-shift-l", ClearFilters, document),
        KeyBinding::new("cmd-alt-l", ClearFilters, document),
        KeyBinding::new("cmd-alt-f", EditFilter, document),
        KeyBinding::new("cmd-alt-i", AnalyzeSelectedColumn, document),
        KeyBinding::new("cmd-alt-r", ToggleRowNumbers, document),
        KeyBinding::new("cmd-f", input::Search, document),
        KeyBinding::new("cmd-g", FindNext, document),
        KeyBinding::new("cmd-shift-g", FindPrevious, document),
        KeyBinding::new("escape", input::Escape, document),
        KeyBinding::new("cmd-shift-e", ExportTable, document),
        KeyBinding::new("left", PreviousPoint, scatter),
        KeyBinding::new("right", NextPoint, scatter),
        KeyBinding::new("enter", RevealPoint, scatter),
        KeyBinding::new("space", RevealPoint, scatter),
    ]);
    for (key, shifted, motion) in [
        ("up", "shift-up", Motion::Up),
        ("down", "shift-down", Motion::Down),
        ("left", "shift-left", Motion::Left),
        ("right", "shift-right", Motion::Right),
        ("home", "shift-home", Motion::FirstRow),
        ("cmd-up", "shift-cmd-up", Motion::FirstRow),
        ("end", "shift-end", Motion::LastRow),
        ("cmd-down", "shift-cmd-down", Motion::LastRow),
        ("cmd-left", "shift-cmd-left", Motion::FirstColumn),
        ("cmd-right", "shift-cmd-right", Motion::LastColumn),
        ("pageup", "shift-pageup", Motion::PageUp),
        ("pagedown", "shift-pagedown", Motion::PageDown),
    ] {
        for (key, extend) in [(key, false), (shifted, true)] {
            cx.bind_keys([KeyBinding::new(key, Navigate { motion, extend }, table)]);
        }
    }
}
