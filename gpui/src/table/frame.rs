use super::*;

impl TableView {
    pub(super) fn build_frame(&mut self, layout: Rc<Layout>, cx: &App) -> Frame {
        let visible: Vec<usize> = layout.columns.iter().map(|c| c.0).collect();
        let rows = layout.vertical.visible_rows();
        let formatting_error = self.data.prefetch(&visible, rows.clone());

        let find = self.find.read(cx);
        let row_y = |row: u64| HEADER_HEIGHT + layout.vertical.row_top(row) as f32;
        let mut matches = vec![];
        let mut text_matches = Vec::new();
        let mut current_match = None;
        let mut columns = Vec::with_capacity(layout.columns.len());
        for &(column, x, width) in &layout.columns {
            let info = &self.data.columns[column];
            let mut cells = Vec::with_capacity(rows.clone().count());
            for row in rows.clone() {
                let (text, null) = match self.data.cell_preview(column, row) {
                    Some(Some(text)) => (text, false),
                    Some(None) => (SharedString::new_static("null"), true),
                    None => {
                        continue;
                    }
                };
                let current = find
                    .current_match()
                    .is_some_and(|hit| hit.column == info.id && hit.display_row == row);
                let first_span = text_matches.len();
                if current || find.matched(info.id, row) {
                    let bounds = Bounds::new(
                        point(px(x), px(row_y(row))),
                        size_(width, self.prefs.row_height),
                    );
                    if current {
                        current_match = Some(bounds);
                    }
                    matches.push(bounds);
                    if info.capabilities.text_match
                        && let Some(session) = find.session()
                        && let Some(Some(source)) = self.data.cached_cell_text(column, row)
                    {
                        let prefix = crate::data::single_line_prefix(
                            source,
                            crate::data::GRID_PREVIEW_BYTES,
                        );
                        text_matches.extend(session.text_match_ranges(source, prefix.len()));
                    }
                }
                cells.push(PaintCell {
                    y: row_y(row),
                    text,
                    missing: null,
                    spans: first_span..text_matches.len(),
                });
            }
            columns.push(PaintColumn {
                x,
                width,
                right_aligned: self.data.is_number(column),
                cells,
            });
        }
        let header = layout
            .columns
            .iter()
            .map(|&(column, x, width)| {
                let id = self.data.columns[column].id;
                let name = &self.data.columns.entry(column).label;
                let name = if name.len() > 1024
                    || name.contains(['\r', '\n', '\u{85}', '\u{2028}', '\u{2029}'])
                {
                    crate::data::preview(name)
                } else {
                    name.clone()
                };
                let sort = self.data.query().sorts.iter().position(|s| s.column == id);
                let label = if let Some(i) = sort {
                    let sorts = &self.data.query().sorts;
                    let arrow = if sorts[i].descending { "▼" } else { "▲" };
                    if sorts.len() > 1 {
                        format!("{name} {arrow}{}", i + 1).into()
                    } else {
                        format!("{name} {arrow}").into()
                    }
                } else {
                    name
                };
                let filtered = self.data.query().filters.iter().any(|f| f.column == id);
                let show = filtered
                    || self.header_hover == Some(column)
                    || self.selection.is_some_and(|s| s.head.column == column);
                (
                    x,
                    width,
                    label,
                    self.data.columns[column].display_type.clone().into(),
                    show.then_some(filtered),
                )
            })
            .collect();
        let gutter = rows
            .clone()
            .map(|row| {
                (
                    row_y(row),
                    SharedString::from(
                        (self.data.source_row(row) + self.prefs.row_number_base).to_string(),
                    ),
                )
            })
            .collect();
        let stripes = rows.filter(|r| r % 2 == 1).map(row_y).collect();
        let cell_bounds = |rows: Range<u64>, cols: Range<usize>| -> Option<Bounds<Pixels>> {
            let x0 = layout.gutter + self.data.columns.widths().take(cols.start).sum::<f32>()
                - layout.scroll_x;
            let x1 = layout.gutter + self.data.columns.widths().take(cols.end).sum::<f32>()
                - layout.scroll_x;
            let y0 = (HEADER_HEIGHT as f64 + layout.vertical.row_top(rows.start)).max(-1e6) as f32;
            let y1 = (HEADER_HEIGHT as f64 + layout.vertical.row_top(rows.end)).min(1e6) as f32;
            (y1 > HEADER_HEIGHT && y0 < layout.body_bottom())
                .then(|| Bounds::from_corners(point(px(x0), px(y0)), point(px(x1), px(y1))))
        };
        let selection = self
            .selection
            .and_then(|s| cell_bounds(s.rows(), s.columns()));
        let head = self.selection.and_then(|s| {
            cell_bounds(s.head.row..s.head.row + 1, s.head.column..s.head.column + 1)
        });
        Frame {
            formatting_error,
            style: self.prefs,
            active_thumb: match self.drag {
                Drag::Thumb { .. } => Some(Thumb::Vertical),
                Drag::HorizontalThumb { .. } => Some(Thumb::Horizontal),
                _ => self.thumb_hover,
            },
            header_hover: self.header_hover,
            show_scrollbars: !cx.should_auto_hide_scrollbars() || self.scrollbar_timer.is_some(),
            hovered_row_y: self.body_pointer.and_then(|point| {
                layout
                    .cell_at(point.x.as_f32(), point.y.as_f32())
                    .map(|cell| row_y(cell.row))
            }),
            thumb: layout.vertical_thumb(),
            layout,
            header,
            gutter,
            columns,
            stripes,
            matches,
            current_match,
            text_matches,
            selection,
            head,
        }
    }
}
