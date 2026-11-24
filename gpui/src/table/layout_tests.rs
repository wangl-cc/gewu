use super::{Cell, Layout, ROW_HEIGHT, VerticalScroll, size_};
use crate::scroll::Anchor;

#[test]
fn click_requires_a_painted_cell_but_drag_extends_to_the_edge() {
    let mut layout = Layout {
        bounds: gpui_kit::Bounds::new(Default::default(), size_(200., 200.)),
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
