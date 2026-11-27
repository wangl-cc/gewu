use gpui_kit::{Axis, Bounds, Pixels, Point, point, px};

use super::{DragUpdate, Geometry, GridEffect, GridModel, HeaderHover};
use crate::{
    actions::Motion,
    table::{
        HEADER_HEIGHT, MIN_COLUMN, PAD, ROW_HEIGHT, SCROLLBAR,
        columns::{ColumnWidth, TableColumns, fixture::columns},
        layout::Layout,
        scroll::VerticalScroll,
        selection::{Cell, Selection},
        size_,
    },
};

const ROWS: u64 = 256;
const COLUMNS: usize = 16;
const WIDTH: f32 = 100.;
const GUTTER: f32 = 40.;

/// A grid laid out as the table would draw it, with the layout committed after each input.
struct Grid {
    columns: TableColumns,
    model: GridModel,
    layout: Layout,
    auto_hide: bool,
    show_scrollbars: bool,
}

impl Grid {
    fn new(width: f32, height: f32, auto_hide: bool) -> Self {
        let columns = columns(COLUMNS, WIDTH);
        let scroll =
            VerticalScroll::new(ROWS, ROW_HEIGHT.into(), f64::from(height - HEADER_HEIGHT));
        let model = GridModel::new(scroll);
        let layout = Layout::new(
            Bounds::new(Point::default(), size_(width, height)),
            GUTTER,
            &columns,
            model.scroll(),
            model.horizontal(),
            auto_hide,
        );
        let mut grid = Self {
            columns,
            model,
            layout,
            auto_hide,
            show_scrollbars: !auto_hide,
        };
        grid.draw();
        grid
    }

    /// Lay out and commit the next frame, as drawing does.
    fn draw(&mut self) {
        let requested = self.model.horizontal();
        self.layout = Layout::new(
            self.layout.bounds,
            GUTTER,
            &self.columns,
            self.model.scroll(),
            requested,
            self.auto_hide,
        );
        self.model.commit(&self.layout, requested);
    }

    fn cell(&self, row: u64, column: usize) -> Point<Pixels> {
        point(
            px(self.layout.column_left(column).unwrap() + 10.),
            px(HEADER_HEIGHT + self.layout.vertical.row_top(row) as f32 + ROW_HEIGHT / 2.),
        )
    }

    fn header(&self, x: f32) -> Point<Pixels> {
        point(px(x), px(HEADER_HEIGHT / 2.))
    }

    fn press(&mut self, at: Point<Pixels>, shift: bool, clicks: usize) -> Option<GridEffect> {
        let geometry = Geometry {
            layout: &self.layout,
            columns: &self.columns,
            auto_hide_scrollbars: self.auto_hide,
        };
        let effect = self
            .model
            .pointer_down(at, shift, clicks, &geometry, self.show_scrollbars);
        self.draw();
        effect
    }

    fn drag(&mut self, to: Point<Pixels>) -> Option<DragUpdate> {
        let geometry = Geometry {
            layout: &self.layout,
            columns: &self.columns,
            auto_hide_scrollbars: self.auto_hide,
        };
        let update = self.model.pointer_drag(to, &geometry);
        if let Some(DragUpdate::Resize { column, width }) = update {
            self.columns.set_width(column, ColumnWidth::Manual(width));
        }
        self.draw();
        update
    }

    fn edge_frame(&mut self) -> Option<DragUpdate> {
        let geometry = Geometry {
            layout: &self.layout,
            columns: &self.columns,
            auto_hide_scrollbars: self.auto_hide,
        };
        let update = self.model.edge_scroll(&geometry);
        self.draw();
        update
    }

    fn key(&mut self, motion: Motion, extend: bool) -> bool {
        let scrolled = self
            .model
            .navigate(motion, extend, ROWS, &self.columns, &self.layout);
        self.draw();
        scrolled
    }

    fn ranges(&self) -> Option<(std::ops::Range<u64>, std::ops::Range<usize>)> {
        self.model.selection().map(|s| (s.rows(), s.columns()))
    }
}

fn single(row: u64, column: usize) -> Option<Selection> {
    Some(Selection::single(Cell { row, column }))
}

#[test]
fn presses_select_cells_shift_extends_from_the_anchor_and_blank_areas_change_nothing() {
    let mut grid = Grid::new(680., 420., false);
    assert_eq!(grid.press(grid.cell(2, 0), false, 1), None);
    assert_eq!(grid.model.selection(), single(2, 0));
    grid.model.pointer_up();
    grid.press(grid.cell(5, 2), true, 1);
    assert_eq!(grid.ranges(), Some((2..6, 0..3)));
    grid.model.pointer_up();

    let mut narrow = Grid::new(680., 420., false);
    narrow.columns = columns(2, WIDTH);
    narrow.model.set_rows(3);
    narrow.draw();
    narrow.press(narrow.cell(0, 0), false, 1);
    narrow.model.pointer_up();
    let below = point(px(GUTTER + 8.), px(HEADER_HEIGHT + 4. * ROW_HEIGHT));
    let right = point(px(narrow.layout.body_right() - 2.), px(60.));
    for blank in [below, right] {
        assert_eq!(narrow.press(blank, false, 1), None);
        assert_eq!(narrow.model.selection(), single(0, 0));
    }
}

#[test]
fn a_double_press_on_a_cell_selects_it_and_asks_to_show_it() {
    let mut grid = Grid::new(680., 420., false);
    let cell = Cell { row: 3, column: 1 };
    assert_eq!(
        grid.press(grid.cell(3, 1), false, 2),
        Some(GridEffect::ShowCell(cell))
    );
    assert_eq!(grid.model.selection(), single(3, 1));
    assert!(!grid.model.is_dragging());
}

#[test]
fn dragging_moves_the_head_and_keeps_the_anchor() {
    let mut grid = Grid::new(680., 420., false);
    grid.press(grid.cell(1, 0), false, 1);
    assert_eq!(grid.drag(grid.cell(4, 1)), Some(DragUpdate::Moved));
    grid.model.pointer_up();
    assert_eq!(grid.ranges(), Some((1..5, 0..2)));
    assert_eq!(grid.drag(grid.cell(6, 2)), None);
    assert_eq!(grid.ranges(), Some((1..5, 0..2)));
}

#[test]
fn a_drag_resting_past_the_bottom_edge_scrolls_each_frame_until_the_last_row() {
    let mut grid = Grid::new(680., 420., false);
    grid.press(grid.cell(0, 0), false, 1);
    let below = point(px(GUTTER + 10.), px(grid.layout.body_bottom() + 30.));
    assert_eq!(grid.drag(below), Some(DragUpdate::EdgeScrolling));
    let mut frames = 0;
    while grid.edge_frame() == Some(DragUpdate::EdgeScrolling) {
        frames += 1;
        assert!(frames < 1000, "edge scrolling must stop at the last row");
    }
    assert_eq!(grid.model.scroll().fraction(), 1.);
    assert_eq!(grid.model.selection().unwrap().head.row, ROWS - 1);
    grid.model.pointer_up();
    assert_eq!(grid.edge_frame(), None);
}

#[test]
fn keys_move_or_extend_the_head_and_reveal_it() {
    let mut grid = Grid::new(680., 420., false);
    grid.press(grid.cell(1, 0), false, 1);
    grid.model.pointer_up();
    grid.key(Motion::Down, true);
    grid.key(Motion::Down, true);
    grid.key(Motion::Right, true);
    assert_eq!(grid.ranges(), Some((1..4, 0..2)));
    grid.key(Motion::Down, false);
    assert_eq!(grid.model.selection(), single(4, 1));
    assert!(grid.key(Motion::PageDown, false));
    let row = grid.model.selection().unwrap().head.row;
    assert_eq!(row, 4 + grid.model.scroll().page_rows());
    assert!(grid.layout.vertical.visible_rows().contains(&row));
    grid.key(Motion::LastRow, false);
    assert_eq!(grid.model.selection().unwrap().head.row, ROWS - 1);
    assert_eq!(grid.model.scroll().fraction(), 1.);
    grid.key(Motion::FirstRow, true);
    assert_eq!(grid.model.selection().unwrap().rows(), 0..ROWS);
    assert_eq!(grid.model.scroll().fraction(), 0.);
    grid.key(Motion::LastColumn, false);
    let right = grid.layout.column_left(COLUMNS - 1).unwrap() + WIDTH;
    assert!(right <= grid.layout.body_right());
    assert!(!grid.key(Motion::LastColumn, false));
}

#[test]
fn header_presses_sort_filter_or_start_a_resize_by_the_part_under_the_pointer() {
    let mut grid = Grid::new(680., 420., false);
    let right = GUTTER + WIDTH;
    assert_eq!(
        grid.press(grid.header(GUTTER + 10.), false, 1),
        Some(GridEffect::CycleSort {
            column: 0,
            multiple: false
        })
    );
    assert_eq!(
        grid.press(grid.header(GUTTER + 10.), true, 1),
        Some(GridEffect::CycleSort {
            column: 0,
            multiple: true
        })
    );
    // The funnel target starts five points left of the glyph and reaches the divider.
    let funnel = Layout::funnel_left(right) - 5.;
    assert_eq!(funnel, right - PAD - 14.);
    assert_eq!(
        grid.press(grid.header(funnel), false, 1),
        Some(GridEffect::EditFilter(0))
    );
    assert!(matches!(
        grid.press(grid.header(funnel - 0.5), false, 1),
        Some(GridEffect::CycleSort { column: 0, .. })
    ));
    assert_eq!(
        grid.press(grid.header(right - 3.), false, 2),
        Some(GridEffect::Autofit(0))
    );
    assert_eq!(grid.press(grid.header(right + 3.), false, 1), None);
    assert!(grid.model.is_dragging());
}

#[test]
fn a_divider_drag_resizes_from_the_press_and_stops_at_the_minimum_width() {
    let mut grid = Grid::new(680., 420., false);
    let right = GUTTER + WIDTH;
    grid.press(grid.header(right), false, 1);
    assert_eq!(
        grid.drag(grid.header(right + 60.)),
        Some(DragUpdate::Resize {
            column: 0,
            width: WIDTH + 60.
        })
    );
    assert_eq!(
        grid.drag(grid.header(GUTTER)),
        Some(DragUpdate::Resize {
            column: 0,
            width: MIN_COLUMN
        })
    );
    assert_eq!(grid.layout.column_left(1), Some(GUTTER + MIN_COLUMN));
}

#[test]
fn wheel_deltas_below_a_point_accumulate_and_shift_scrolls_sideways() {
    let mut grid = Grid::new(680., 420., false);
    for _ in 0..40 {
        grid.model
            .wheel(0., -0.5, false, &grid.columns, &grid.layout);
        grid.draw();
    }
    let offset = grid.model.scroll().position() * f64::from(ROW_HEIGHT);
    assert!((offset - 20.).abs() < 1e-6);
    grid.model
        .wheel(0., -30., true, &grid.columns, &grid.layout);
    grid.draw();
    assert_eq!(grid.layout.scroll_x, 30.);
    grid.model
        .wheel(10_000., 0., false, &grid.columns, &grid.layout);
    grid.draw();
    assert_eq!(grid.layout.scroll_x, 0.);
}

#[test]
fn thumbs_follow_the_pointer_and_track_presses_center_the_thumb_there() {
    let mut grid = Grid::new(680., 420., false);
    let bar = grid.layout.scrollbar(Axis::Vertical).unwrap();
    grid.press(bar.thumb.center(), false, 1);
    grid.drag(point(bar.thumb.center().x, bar.track.bottom()));
    assert_eq!(grid.model.scroll().fraction(), 1.);
    let middle = (bar.track.top() + bar.track.bottom()) / 2.;
    grid.drag(point(bar.thumb.center().x, middle));
    assert!((grid.model.scroll().fraction() - 0.5).abs() < 0.01);
    grid.model.pointer_up();

    grid.press(grid.cell(0, 0), false, 1);
    grid.model.pointer_up();
    let selected = grid.model.selection();
    let vertical = grid.layout.scrollbar(Axis::Vertical).unwrap().track;
    let horizontal = grid.layout.scrollbar(Axis::Horizontal).unwrap().track;
    grid.press(vertical.bottom_right() - point(px(1.), px(1.)), false, 1);
    assert_eq!(grid.model.scroll().fraction(), 1.);
    grid.press(horizontal.bottom_right() - point(px(1.), px(1.)), false, 1);
    assert_eq!(grid.layout.scroll_x, grid.layout.horizontal_overflow());
    assert_eq!(grid.model.selection(), selected);
}

#[test]
fn hidden_overlay_tracks_pass_presses_through_unless_a_shown_thumb_was_hovered() {
    let mut grid = Grid::new(680., 420., true);
    let track = grid.layout.scrollbar(Axis::Vertical).unwrap().track;
    let inside = point(track.center().x, grid.cell(2, 0).y);
    assert!(grid.model.pointer_hover(inside, &grid.layout, false));
    assert_eq!(grid.model.active_thumb(), None);
    assert!(grid.press(inside, false, 1).is_none());
    assert_eq!(grid.model.selection().unwrap().head.row, 2);
    grid.model.pointer_up();
    // While the tracks flash after scrolling, hovering one keeps it hittable once hidden.
    assert!(grid.model.pointer_hover(inside, &grid.layout, true));
    assert_eq!(grid.model.active_thumb(), Some(Axis::Vertical));
    grid.press(inside, false, 1);
    assert!(grid.model.is_dragging());
    assert!(grid.model.scroll().fraction() > 0.);
    assert_eq!(grid.model.selection().unwrap().head.row, 2);
}

#[test]
fn hover_reports_only_changes_that_are_drawn() {
    let mut grid = Grid::new(680., 420., false);
    let cell = grid.cell(3, 0);
    assert!(grid.model.pointer_hover(cell, &grid.layout, true));
    assert!(
        !grid
            .model
            .pointer_hover(cell + point(px(20.), px(2.)), &grid.layout, true)
    );
    assert!(
        grid.model
            .pointer_hover(grid.cell(4, 0), &grid.layout, true)
    );
    assert_eq!(grid.model.hovered_cell(&grid.layout).unwrap().row, 4);
    grid.model.scroll_to_row(1);
    grid.draw();
    assert_eq!(grid.model.hovered_cell(&grid.layout).unwrap().row, 5);
    let right = GUTTER + WIDTH;
    assert!(
        grid.model
            .pointer_hover(grid.header(right), &grid.layout, true)
    );
    assert_eq!(grid.model.header_hover(), Some(HeaderHover::Divider(0)));
    assert!(
        grid.model
            .pointer_hover(grid.header(right - PAD - 10.), &grid.layout, true)
    );
    assert_eq!(grid.model.header_hover(), Some(HeaderHover::Column(0)));
    assert!(grid.model.pointer_left());
    assert!(!grid.model.pointer_left());
    assert_eq!(grid.model.header_hover(), None);
}

#[test]
fn always_visible_tracks_reserve_their_space_and_meet_at_the_corner() {
    let grid = Grid::new(680., 420., false);
    let l = &grid.layout;
    assert_eq!(
        l.vertical.viewport(),
        f64::from(420. - HEADER_HEIGHT - SCROLLBAR)
    );
    let v = l.scrollbar(Axis::Vertical).unwrap().track;
    let h = l.scrollbar(Axis::Horizontal).unwrap().track;
    assert_eq!((v.right(), v.bottom()), (px(680.), px(420.)));
    assert_eq!((h.bottom(), h.right()), (px(420.), v.left()));
    let corner = v.bottom_right() - point(px(1.), px(1.));
    assert!(l.cell_at(corner.x.as_f32(), corner.y.as_f32()).is_none());
}
