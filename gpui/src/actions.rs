//! Keyboard commands are scoped to either the document or the focused grid.
use gpui_kit::{Action, App, KeyBinding, base::input};

gpui_kit::actions!(gewu, [
    FilterSelection,
    ClearFilters,
    FindNext,
    FindPrevious,
    AnalyzeSelectedColumn,
    ShowChartInspector,
    ToggleRowNumbers,
    ToggleImportHeader,
    ShowInspector,
    EditFilter,
    ManageColumns,
    EditSort,
    ExportTable
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

#[derive(Action, Clone, Debug, Default, PartialEq, Eq)]
#[action(namespace = gewu, no_json)]
pub(crate) struct RevealScatterPoint {
    pub chart: gpui_kit::EntityId,
    pub source_row: gewu_core::SourceRow,
}

pub(crate) fn init(cx: &mut App) {
    let table = Some("GewuGrid");
    let document = Some("GewuDocument");
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
