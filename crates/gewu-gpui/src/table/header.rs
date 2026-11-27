//! Column measurement and hover targets share the canvas's actual bounds.
use super::{columns::VisibleCol, *};

const SAMPLE_ROWS: u64 = 128;
const INITIAL_MAX: f32 = 320.;
const FIT_MAX: f32 = 800.;
pub(super) const NAME_SIZE: f32 = 12.;
pub(super) const TYPE_SIZE: f32 = 10.5;
pub(super) const SORT_SIZE: f32 = 10.;
pub(super) const SORT_GAP: f32 = 3.;
pub(super) const HEADER_INSET: f32 = 2. * PAD + 16.;

struct HeaderTip(SharedString);
impl Render for HeaderTip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = crate::theme::GewuPalette::get(cx);
        div()
            .max_w(px(400.))
            .px_2()
            .py_1()
            .rounded(px(10.))
            .bg(colors.elevated)
            .shadow(colors.elevated_shadows())
            .text_color(colors.foreground)
            .border_1()
            .border_color(colors.elevated_border)
            .text_sm()
            .child(self.0.clone())
    }
}

/// A sorted column's header marker: its direction, and its priority when several columns sort.
pub(super) fn sort_marker(descending: bool, priority: usize, rules: usize) -> SharedString {
    let arrow = if descending { "▼" } else { "▲" };
    if rules > 1 {
        format!("{arrow}{}", priority + 1).into()
    } else {
        SharedString::new_static(arrow)
    }
}

impl TableView {
    pub(super) fn hover_target(&self, l: &Layout) -> Option<AnyElement> {
        let index = match self.grid.header_hover()? {
            HeaderHover::Divider(column) => {
                let (start, end) = l.divider_band(column)?;
                return Some(
                    div()
                        .relative()
                        .size_full()
                        .child(
                            div()
                                .absolute()
                                .left(px(start))
                                .w(px(end - start))
                                .h(px(HEADER_HEIGHT))
                                .cursor(CursorStyle::ResizeColumn),
                        )
                        .into_any_element(),
                );
            }
            HeaderHover::Column(column) => column,
        };
        let laid = l.laid(index)?;
        let column = &self.data.columns[index];
        let start = laid.x.max(l.gutter);
        let end = laid.right().min(l.body_right());
        if start >= end {
            return None;
        }
        let details = self
            .data
            .query()
            .filters
            .iter()
            .find(|f| f.column == column.id)
            .map(|f| {
                format!(
                    "{} · {}",
                    column.technical_type,
                    crate::panels::filter_description(&f.filter)
                )
            })
            .unwrap_or_else(|| column.technical_type.clone());
        let mut tip = format!("{} · {details}", crate::table::data::preview(&column.name));
        let sorts = &self.data.query().sorts;
        if let Some((i, sort)) = sorts
            .iter()
            .enumerate()
            .find(|(_, s)| s.column == column.id)
        {
            let direction = if sort.descending {
                "Descending"
            } else {
                "Ascending"
            };
            tip.push_str(&format!(" · {direction}"));
            if sorts.len() > 1 {
                tip.push_str(&format!(" ({} of {})", i + 1, sorts.len()));
            }
        }
        let tip: SharedString = tip.into();
        let area = div()
            .id(("header-tip", index.index()))
            .absolute()
            .left(px(start))
            .w(px(end - start))
            .h(px(HEADER_HEIGHT))
            .tooltip(move |_, cx| cx.new(|_| HeaderTip(tip.clone())).into());
        Some(div().relative().size_full().child(area).into_any_element())
    }

    pub(super) fn initial_width(&mut self, column: VisibleCol, window: &Window) -> f32 {
        self.measure_width(column, 0, INITIAL_MAX, window)
    }

    pub(super) fn autofit(&mut self, column: VisibleCol, window: &Window) {
        let width = self.measure_width(column, self.layout.vertical.anchor().row, FIT_MAX, window);
        self.data
            .columns
            .set_width(column, ColumnWidth::Measured(width));
    }

    fn measure_width(&mut self, column: VisibleCol, start: u64, max: f32, window: &Window) -> f32 {
        let mut font = window.text_style().font();
        font.features = crate::theme::tabular_digits();
        let measure = |text: &str, font_size: f32, weight| {
            // A very long first value can determine the cap without shaping the entire cell.
            let preview = crate::table::data::single_line_prefix(
                text,
                crate::table::data::GRID_PREVIEW_BYTES,
            );
            let run = TextRun {
                len: preview.len(),
                font: Font {
                    weight,
                    ..font.clone()
                },
                color: window.text_style().color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(SharedString::new(preview), px(font_size), &[run], None)
                .width
                .as_f32()
        };
        let rows = start..start.saturating_add(SAMPLE_ROWS).min(self.data.row_count());
        self.data.prefetch([column], rows.clone());
        // Size for a future sort of any priority so sorting does not reflow measured columns.
        let last = self.data.snapshot().dataset().schema().fields().len();
        let sort = [false, true].into_iter().fold(0.0f32, |width, descending| {
            let marker = sort_marker(descending, last.saturating_sub(1), last);
            width.max(measure(&marker, SORT_SIZE, FontWeight::NORMAL))
        });
        let mut width = (measure(
            &self.data.columns.entry(column).label,
            NAME_SIZE,
            FontWeight::MEDIUM,
        ) + sort
            + SORT_GAP)
            .max(measure(
                &self.data.columns[column].display_type,
                TYPE_SIZE,
                FontWeight::NORMAL,
            ))
            + HEADER_INSET;
        for row in rows {
            let cell = Cell {
                row: gewu_core::ViewRow(row),
                column,
            };
            if let Some(Some(text)) = self.data.cell_preview(cell) {
                width =
                    width.max(measure(&text, self.prefs.font_size, FontWeight::NORMAL) + 2. * PAD);
            }
            if width >= max {
                break;
            }
        }
        width.ceil().clamp(MIN_COLUMN, max)
    }
}
