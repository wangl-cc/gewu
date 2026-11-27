use gpui_kit::{Axis, Bounds, Point};

use super::{Cell, HEADER_HEIGHT, Layout, ROW_HEIGHT, SCROLLBAR, VerticalScroll, size_};
use crate::table::{
    PAD,
    columns::fixture::columns,
    layout::{HeaderPart, HorizontalScroll},
    scroll::Anchor,
};

#[test]
fn click_requires_a_painted_cell_but_drag_extends_to_the_edge() {
    let mut layout = Layout {
        bounds: gpui_kit::Bounds::new(Default::default(), size_(200., 200.)),
        viewport: size_(200., 200.),
        gutter: 24.,
        scroll_x: 194.,
        content_width: 100.,
        // Column 2 starts behind the frozen gutter.
        columns: vec![(2, -10., 80.), (3, 70., 60.)],
        vertical: VerticalScroll::new(2, ROW_HEIGHT.into(), 168.),
    };
    let first = Cell { row: 0, column: 2 };
    let last = Cell { row: 1, column: 3 };
    assert_eq!(layout.column_edges(2..3), Some((-10., 70.)));
    assert_eq!(layout.column_edges(0..8), Some((-10., 130.)));
    assert_eq!(layout.column_edges(4..5), None);
    assert_eq!(layout.cell_at(24., 32.), Some(first));
    assert_eq!(layout.cell_at(70., 64.), Some(last));
    for (x, y) in [(23., 32.), (24., 31.), (130., 48.), (80., 96.)] {
        assert_eq!(layout.cell_at(x, y), None);
    }
    assert_eq!(layout.drag_cell_at(-10., 0.), Some(first));
    assert_eq!(layout.drag_cell_at(300., 300.), Some(last));

    layout.vertical = VerticalScroll::new(100, ROW_HEIGHT.into(), 168.);
    layout.vertical.set_anchor(Anchor {
        row: 45,
        offset: 12.,
    });
    layout.columns[1].2 = 180.;
    assert_eq!(layout.cell_at(70., 52.), Some(Cell { row: 46, column: 3 }));
    assert_eq!(layout.cell_at(199., 52.), Some(Cell { row: 46, column: 3 }));
    assert_eq!(layout.cell_at(200., 52.), None);
    assert_eq!(layout.cell_at(70., 200.), None);
    assert_eq!(
        layout.drag_cell_at(300., 300.),
        Some(Cell { row: 50, column: 3 })
    );
}

#[test]
fn header_parts_share_one_funnel_and_divider_geometry() {
    let layout = Layout {
        bounds: Bounds::new(Default::default(), size_(400., 200.)),
        viewport: size_(400., 200.),
        gutter: 20.,
        scroll_x: 0.,
        content_width: 200.,
        columns: vec![(0, 20., 100.), (1, 120., 100.)],
        vertical: VerticalScroll::new(2, ROW_HEIGHT.into(), 168.),
    };
    let right = 120.;
    assert_eq!(Layout::funnel_left(right), right - PAD - 9.);
    for (x, part) in [
        (19., None),
        (21., Some(HeaderPart::Title(0))),
        (right - PAD - 14.5, Some(HeaderPart::Title(0))),
        (right - PAD - 14., Some(HeaderPart::Funnel(0))),
        (right - 4.5, Some(HeaderPart::Funnel(0))),
        (right - 4., Some(HeaderPart::Divider(0))),
        (right + 4., Some(HeaderPart::Divider(0))),
        (right + 4.5, Some(HeaderPart::Title(1))),
        (224., Some(HeaderPart::Divider(1))),
        (224.5, None),
    ] {
        assert_eq!(layout.header_part_at(x), part, "at {x}");
    }
    assert_eq!(layout.divider_band(0), Some((116., 124.)));
    assert_eq!(layout.divider_band(1), Some((216., 224.)));
}

#[test]
fn scrollbar_lanes_appear_only_for_overflow_including_overflow_induced_by_the_other_axis() {
    const ROWS: u64 = 256;
    const WIDTH: f32 = 100.;
    const GUTTER: f32 = 40.;
    let columns = columns(16, WIDTH);
    let scroll = VerticalScroll::new(ROWS, ROW_HEIGHT.into(), 0.);
    let fit_w = 16. * WIDTH + GUTTER;
    let fit_h = HEADER_HEIGHT + ROWS as f32 * ROW_HEIGHT;
    for (dw, dh, horizontal, vertical) in [
        (0., 0., false, false),
        (16., -1., false, true),
        (-1., 16., true, false),
        (6., -1., true, true),
        (-1., 6., true, true),
    ] {
        let outer = size_(fit_w + dw, fit_h + dh);
        let bounds = Bounds::new(Point::default(), outer);
        let layout = |auto_hide| {
            Layout::new(
                bounds,
                GUTTER,
                &columns,
                &scroll,
                HorizontalScroll::Offset(0.),
                auto_hide,
            )
        };
        let always = layout(false);
        assert_eq!(always.scrollbar(Axis::Horizontal).is_some(), horizontal);
        assert_eq!(always.scrollbar(Axis::Vertical).is_some(), vertical);
        let reserved = size_(
            if vertical { SCROLLBAR } else { 0. },
            if horizontal { SCROLLBAR } else { 0. },
        );
        assert_eq!(always.viewport, outer - reserved);
        let overlay = layout(true);
        assert_eq!(overlay.viewport, outer);
        assert_eq!(overlay.scrollbar(Axis::Horizontal).is_some(), dw < 0.);
        assert_eq!(overlay.scrollbar(Axis::Vertical).is_some(), dh < 0.);
    }
}
