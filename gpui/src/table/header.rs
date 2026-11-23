//! Column measurement and hover targets share the canvas's actual bounds.
use super::*;

const SAMPLE_ROWS: u64 = 128;
const INITIAL_MAX: f32 = 320.;
const FIT_MAX: f32 = 800.;

struct HeaderTip(SharedString);
impl Render for HeaderTip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = base::Theme::global(cx).tokens.colors;
        div()
            .max_w(px(400.))
            .px_2()
            .py_1()
            .rounded_md()
            .bg(colors.surface)
            .text_color(colors.foreground)
            .border_1()
            .border_color(colors.border)
            .text_sm()
            .child(self.0.clone())
    }
}

impl TableView {
    pub(super) fn hover_target(&self, l: &Layout) -> Option<AnyElement> {
        let index = self.header_hover?;
        let &(_, left, width) = l.columns.iter().find(|c| c.0 == index)?;
        let column = &self.data.columns[index];
        let start = left.max(l.gutter);
        let end = (left + width).min(l.body_right());
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
        let mut tip = format!("{} · {details}", crate::data::preview(&column.name));
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
        let mut area = div()
            .id(("header-tip", index))
            .absolute()
            .left(px(start))
            .w(px(end - start))
            .h(px(HEADER_HEIGHT))
            .tooltip(move |_, cx| cx.new(|_| HeaderTip(tip.clone())).into());
        if end >= left + width - RESIZE_SLOP {
            area = area.child(
                div()
                    .absolute()
                    .right_0()
                    .w(px(RESIZE_SLOP * 2.))
                    .h_full()
                    .cursor(CursorStyle::ResizeColumn),
            );
        }
        Some(div().relative().size_full().child(area).into_any_element())
    }

    pub(super) fn size_new_columns(&mut self, l: &Layout, window: &Window) -> bool {
        let mut changed = false;
        for &(index, ..) in &l.columns {
            if self.data.columns.entry(index).width == ColumnWidth::Unmeasured {
                self.fit_width(index, 0, INITIAL_MAX, window);
                changed = true;
            }
        }
        changed
    }

    pub(super) fn autofit(&mut self, column: usize, window: &Window) {
        self.fit_width(column, self.layout.vertical.anchor().row, FIT_MAX, window);
    }

    fn fit_width(&mut self, column: usize, start: u64, max: f32, window: &Window) {
        let mut font = window.text_style().font();
        font.features = crate::theme::tabular_digits();
        let measure = |text: &str| {
            // A very long first value can determine the cap without shaping the entire cell.
            let limit = text.floor_char_boundary(1024.min(text.len()));
            let preview = &text[..limit];
            let preview = preview.lines().next().unwrap_or("");
            let run = TextRun {
                len: preview.len(),
                font: font.clone(),
                color: window.text_style().color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(
                    SharedString::new(preview),
                    px(self.prefs.font_size),
                    &[run],
                    None,
                )
                .width
                .as_f32()
        };
        let rows = start..start.saturating_add(SAMPLE_ROWS).min(self.data.row_count());
        self.data.prefetch(&[column], rows.clone());
        let mut width = measure(&self.data.columns.entry(column).label)
            .max(measure(&self.data.columns[column].display_type))
            + 28.;
        for row in rows {
            if let Some(Some(text)) = self.data.cell_preview(column, row) {
                width = width.max(measure(&text));
            }
            if width >= max {
                break;
            }
        }
        self.data.columns.entry_mut(column).width =
            ColumnWidth::Measured((width + 2. * PAD).ceil().clamp(MIN_COLUMN, max));
    }
}
