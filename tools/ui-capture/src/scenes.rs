use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, bail};
use gpui_kit::{MouseButton, Pixels, PlatformTextSystem, Role, Size, px, size};

use crate::session::{CaptureSession, CaptureTarget};

pub const GROUPS: &[&str] = &[
    "overview",
    "facets",
    "range",
    "chart",
    "columns",
    "cell",
    "preferences",
    "corrupt",
    "large",
    "truncated",
];

pub fn run_group(
    group: &str,
    dataset: &Path,
    text_system: Arc<dyn PlatformTextSystem>,
    output: PathBuf,
    target: &CaptureTarget,
) -> Result<()> {
    let open = |window_size: Option<Size<Pixels>>| {
        CaptureSession::open(
            dataset,
            window_size,
            text_system.clone(),
            output.clone(),
            target.clone(),
        )
    };
    match group {
        "overview" => overview(&mut open(None)?),
        "facets" => facets(&mut open(None)?, &mut open(Some(size(px(680.), px(580.))))?),
        "range" => range(&mut open(Some(size(px(680.), px(580.))))?),
        "chart" => chart(&mut open(None)?),
        "columns" => columns(&mut open(None)?),
        "cell" => cell(&mut open(None)?),
        "preferences" => preferences(&mut open(None)?),
        "corrupt" => corrupt(&mut open(None)?),
        "large" => large(&mut open(None)?),
        "truncated" if !target.is_high_contrast() => truncated(&mut open(None)?),
        "truncated" => Ok(()),
        other => bail!("unknown scene group {other:?}"),
    }
}

fn overview(s: &mut CaptureSession) -> Result<()> {
    s.click_cell("Station", 1, MouseButton::Left)?;
    s.capture("01-document", "Inspect column", None, false, None, None)?;
    s.key("cmd-f")?;
    s.input("survey")?;
    s.capture(
        "03-find-substring",
        "Match 1 of 104",
        None,
        false,
        None,
        None,
    )
}

fn facets(wide: &mut CaptureSession, narrow: &mut CaptureSession) -> Result<()> {
    wide.click_cell("Station", 0, MouseButton::Left)?;
    wide.action("filter")?;
    wide.capture("10-value-filter", "Deselect all", None, false, None, None)?;

    narrow.click_cell("Station", 0, MouseButton::Left)?;
    narrow.action("filter")?;
    narrow.click_label("Deselect all", None)?;
    narrow.click_label("Blackwall, 16 rows", None)?;
    narrow.click_label("Apply", None)?;
    narrow.click_label("1 filtered", None)?;
    narrow.capture(
        "16b-filter-summary",
        "Show column Station",
        None,
        false,
        None,
        None,
    )
}

fn range(s: &mut CaptureSession) -> Result<()> {
    s.click_cell("Water temperature", 0, MouseButton::Left)?;
    s.action("filter")?;
    if s.find_by(|el| el.label() == Some("Display options"), None)?
        .is_none()
    {
        s.click_label("Range", None)?;
    }
    let start = s
        .require_by(
            "[12.7,",
            |el| el.label().is_some_and(|l| l.starts_with("[12.7,")),
            None,
        )?
        .bounds()
        .center();
    let finish = s
        .require_by(
            "15.6) · 4 rows",
            |el| el.label().is_some_and(|l| l.ends_with("15.6) · 4 rows")),
            None,
        )?
        .bounds()
        .center();
    s.drag(start, finish)?;
    s.capture(
        "14-range-filter",
        "Display options",
        None,
        false,
        None,
        None,
    )
}

fn chart(s: &mut CaptureSession) -> Result<()> {
    s.click_cell("Water temperature", 0, MouseButton::Left)?;
    s.action("analyze")?;
    s.action("chart")?;

    for state in ["off", "on"] {
        if state == "on" {
            s.click_label("Log Y", None)?;
        }
        // Row 81 of the fixture has temperature 16.4 on a 10.2–19.2 domain.
        let point = s.scatter_point(80, 96, 16.4, 10.2, 19.2, state == "on")?;
        s.click_point(point, MouseButton::Left)?;

        let expected = state == "on";
        let mut verify = |session: &mut CaptureSession| -> Result<()> {
            let elements = session.elements()?;
            if !elements
                .iter()
                .any(|el| el.label() == Some("Scatter plot, 96 points, row 81 selected"))
            {
                bail!("missing selected scatter point for row 81");
            }
            let checked = elements
                .iter()
                .find(|el| el.label() == Some("Log Y"))
                .and_then(|el| el.checked());
            if checked != Some(expected) {
                bail!("unexpected Log Y checked state {checked:?}");
            }
            Ok(())
        };
        s.capture(
            &format!("log-y-{state}"),
            "Log Y",
            None,
            false,
            None,
            Some(&mut verify),
        )?;
    }
    Ok(())
}

fn columns(s: &mut CaptureSession) -> Result<()> {
    let mut header_menu = |session: &mut CaptureSession| -> Result<()> {
        session.click_header("Source", MouseButton::Right)
    };
    header_menu(s)?;
    s.capture(
        "18-header-menu",
        "Sort Ascending",
        None,
        false,
        Some(&mut header_menu),
        None,
    )?;
    s.click_label("Sort Ascending", None)?;
    s.require_label("1 sorted", None)?;
    s.action("columns")?;
    s.click_label("Find column", None)?;
    s.input("Water")?;
    s.capture(
        "19b-visible-columns",
        "Hide matching",
        None,
        false,
        None,
        None,
    )?;
    s.click_label("Done", None)?;
    s.action("columns")?;
    s.click_label("Source", Some(Role::CheckBox))?;
    s.capture(
        "20-hide-confirmation",
        "Hide",
        Some(Role::Button),
        false,
        None,
        None,
    )?;
    s.click_label("Cancel", None)?;
    s.click_label("Done", None)?;
    s.action("sort")?;
    s.click_label("Add sort column…", None)?;
    s.click_label("Find column", None)?;
    s.input("Water")?;
    s.click_label("Water temperature", Some(Role::Label))?;

    let mut verify = |session: &mut CaptureSession| -> Result<()> {
        let mut rules: Vec<_> = session
            .elements()?
            .into_iter()
            .filter(|el| el.role() == Some(Role::ComboBox) && el.label() == Some("Sort column"))
            .collect();
        rules.sort_by_key(|a| a.bounds().origin.y);
        let cols: Vec<_> = rules.iter().filter_map(|el| el.value()).collect();
        if cols != ["Source", "Water temperature"] {
            bail!("unexpected sort rule priority: {cols:?}");
        }
        Ok(())
    };
    s.capture(
        "21-sort",
        "2 sorted",
        Some(Role::Button),
        false,
        None,
        Some(&mut verify),
    )
}

fn cell(s: &mut CaptureSession) -> Result<()> {
    let mut cell_menu = |session: &mut CaptureSession| -> Result<()> {
        session.click_cell("Remarks", 1, MouseButton::Right)
    };
    cell_menu(s)?;
    s.capture(
        "23-cell-menu",
        "View Cell…",
        None,
        false,
        Some(&mut cell_menu),
        None,
    )?;
    s.click_label("View Cell…", None)?;
    s.capture(
        "24-cell-preview",
        "Copy cell value",
        None,
        false,
        None,
        None,
    )
}

fn preferences(s: &mut CaptureSession) -> Result<()> {
    s.open_settings()?;
    s.capture("29-settings", "Show row numbers", None, false, None, None)
}

fn corrupt(s: &mut CaptureSession) -> Result<()> {
    s.click_label("Technical details", None)?;
    s.capture(
        "30b-open-error",
        "Technical details",
        None,
        false,
        None,
        None,
    )
}

fn large(s: &mut CaptureSession) -> Result<()> {
    if s.table_dimensions() != Some((2_000_000, 28)) {
        bail!("wrong Parquet fixture: {:?}", s.table_dimensions());
    }
    s.key("cmd-f")?;
    s.input("city")?;
    s.capture(
        "31b-wide-find",
        "Match 1 of 2,000,000",
        None,
        false,
        None,
        None,
    )
}

fn truncated(s: &mut CaptureSession) -> Result<()> {
    let bounds = s.header_bounds("Remarks")?;
    let y = bounds.center().y;
    s.drag(
        gpui_kit::point(bounds.right(), y),
        gpui_kit::point(bounds.left() + px(76.0), y),
    )?;
    let width = s.header_bounds("Remarks")?.size.width.as_f32();
    if width.round() != 76.0 {
        bail!("column was not narrowed to 76 pt (got {width})");
    }
    s.key("cmd-f")?;
    s.input("transect")?;
    s.capture(
        "03b-truncated-ellipsis",
        "Match 1 of 16",
        None,
        true,
        None,
        None,
    )
}
