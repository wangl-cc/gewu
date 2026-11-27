//! Building one frame of the grid from the committed layout: the visible rows, cell text,
//! header markers, Find spans and selection outlines. The frame is immutable and painted once.
use super::{columns::VisibleCol, layout::LaidColumn, *};

impl TableView {
    pub(super) fn build_frame(&mut self, layout: Rc<Layout>, cx: &App) -> Frame {
        let visible = layout.columns.iter().map(|laid| laid.column);
        let visible_rows = layout.vertical.visible_rows();
        let formatting_error = self
            .data
            .prefetch(visible, visible_rows.clone())
            .map(|error| crate::ui::errors::message(error).into());

        let find = self.find.read(cx);
        let row_y = |row: u64| HEADER_HEIGHT + layout.vertical.row_top(row) as f32;
        let sources = self.data.view().source_rows_in(visible_rows.clone());
        assert_eq!(
            sources.len() as u64,
            visible_rows.end - visible_rows.start,
            "the viewport must fit the displayed view"
        );
        let selection = self.grid.selection();
        let selected_rows = selection.map(|s| s.rows());
        let rows: Vec<_> = visible_rows
            .zip(sources)
            .map(|(row, source)| FrameRow {
                display: gewu_core::ViewRow(row),
                source,
                y: row_y(row),
                label: (source.0 + self.prefs.row_number_base).to_string().into(),
                selected: selected_rows
                    .as_ref()
                    .is_some_and(|selected| selected.contains(&row)),
            })
            .collect();
        let mut text_matches = Vec::new();
        let mut current_match = None;
        let mut columns = Vec::with_capacity(layout.columns.len());
        for &LaidColumn { column, x, width } in &layout.columns {
            let info = &self.data.columns[column];
            let mut cells = Vec::with_capacity(rows.len());
            for row in &rows {
                let cell = Cell {
                    row: row.display,
                    column,
                };
                let (text, null) = match self.data.cell_preview(cell) {
                    Some(Some(text)) => (text, false),
                    Some(None) => (SharedString::new_static("null"), true),
                    None => {
                        continue;
                    }
                };
                let current = find
                    .current_match()
                    .is_some_and(|hit| hit.column == info.id && hit.display_row == row.display);
                let first_span = text_matches.len();
                if current {
                    current_match = Some(Bounds::new(
                        point(px(x), px(row.y)),
                        size_(width, self.prefs.row_height),
                    ));
                }
                let data = &self.data;
                find.cell_spans(
                    info.id,
                    row.display,
                    info.capabilities.text_match,
                    || {
                        let source = data.cached_cell_text(cell)??;
                        let preview = crate::table::data::single_line_prefix(
                            source,
                            crate::table::data::GRID_PREVIEW_BYTES,
                        );
                        Some((source, preview))
                    },
                    text.len(),
                    &mut text_matches,
                );
                cells.push(PaintCell {
                    y: row.y,
                    text,
                    missing: null,
                    current_match: current,
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
            .map(|&LaidColumn { column, .. }| {
                let id = self.data.columns[column].id;
                let name = &self.data.columns.entry(column).label;
                let name = if crate::table::data::fits_one_line(name) {
                    name.clone()
                } else {
                    crate::table::data::preview(name)
                };
                let sorts = &self.data.query().sorts;
                let sort = sorts
                    .iter()
                    .position(|s| s.column == id)
                    .map(|i| header::sort_marker(sorts[i].descending, i, sorts.len()));
                let filtered = self.data.query().filters.iter().any(|f| f.column == id);
                let selected = selection.is_some_and(|s| s.columns().contains(&column));
                let show = filtered
                    || self.grid.header_hover() == Some(HeaderHover::Column(column))
                    || selected;
                PaintHeader {
                    name,
                    kind: self.data.columns[column].display_type.clone().into(),
                    sort,
                    funnel: show.then_some(filtered),
                    selected,
                }
            })
            .collect();
        let cell_bounds = |rows: Range<u64>, cols: Range<VisibleCol>| -> Option<Bounds<Pixels>> {
            let (x0, x1) = layout.column_edges(cols)?;
            let y0 = (HEADER_HEIGHT as f64 + layout.vertical.row_top(rows.start)).max(-1e6) as f32;
            let y1 = (HEADER_HEIGHT as f64 + layout.vertical.row_top(rows.end)).min(1e6) as f32;
            (y1 > HEADER_HEIGHT && y0 < layout.body_bottom())
                .then(|| Bounds::from_corners(point(px(x0), px(y0)), point(px(x1), px(y1))))
        };
        let head = selection.and_then(|s| {
            let head = Selection::single(s.head);
            cell_bounds(head.rows(), head.columns())
        });
        let hovered_row_y = self
            .grid
            .hovered_cell(&layout)
            .map(|cell| row_y(cell.row.0));
        let selection = selection.and_then(|s| cell_bounds(s.rows(), s.columns()));
        Frame {
            formatting_error,
            hovered_row_y,
            style: self.prefs,
            active_thumb: self.grid.active_thumb(),
            header_hover: self.grid.header_hover(),
            show_scrollbars: self.show_scrollbars(cx),
            layout,
            header,
            rows,
            columns,
            current_match,
            text_matches,
            selection,
            head,
        }
    }
}
