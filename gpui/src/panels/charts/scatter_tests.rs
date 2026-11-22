use std::sync::Arc;

use gewu_core::{AxisScale, ScatterAxisKind, ScatterPoint, ScatterResult};
use gpui_kit::{
    AppContext, Bounds, Modifiers, Point, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, point, px, size, test::TestWindowExt,
};

use super::ScatterChart;

#[test]
fn hovering_the_plotted_position_reads_the_original_row_and_exact_text() {
    let sample = Arc::new(ScatterResult {
        x_axis_kind: ScatterAxisKind::Number,
        y_axis_kind: ScatterAxisKind::Number,
        eligible_rows: 2,
        non_positive_rows: 0,
        points: vec![
            ScatterPoint {
                source_row: 4,
                x: 4.,
                y: 2.,
                x_label: "4".into(),
                y_label: "2.000000000000001".into(),
            },
            ScatterPoint {
                source_row: 8,
                x: 8.,
                y: 10.,
                x_label: "8".into(),
                y_label: "10".into(),
            },
        ],
    });
    let app = TestAppContext::single();
    let (handle, chart) = app.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Point::default(),
                    size(px(400.), px(380.)),
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|cx| {
                    ScatterChart::new(
                        sample.clone(),
                        AxisScale::Linear,
                        AxisScale::Linear,
                        None,
                        "Score",
                        cx,
                    )
                })
            },
        )
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &app);
    ui.update(|w, cx| w.render_frame(cx));
    let bounds = ui.update(|w, _| w.find("scatter-plot").bounds());
    let first = point(bounds.left() + px(51.), bounds.bottom() - px(23.));
    ui.simulate_mouse_move(first, None, Modifiers::default());
    ui.run_until_parked();
    app.read(|cx| {
        let c = chart.read(cx);
        assert_eq!(c.hover, Some(0));
        assert_eq!(c.sample.points[0].source_row, 4);
        assert_eq!(c.sample.points[0].y_label, "2.000000000000001");
    });
    ui.update(|w, cx| {
        w.drag(
            first,
            point(bounds.right() - px(9.), bounds.top() + px(9.)),
            cx,
        )
    });
    ui.run_until_parked();
    app.read(|cx| assert!(Arc::ptr_eq(&chart.read(cx).sample, &sample)));
    ui.simulate_mouse_move(point(px(1.), bounds.bottom()), None, Modifiers::default());
    ui.run_until_parked();
    app.read(|cx| assert_eq!(chart.read(cx).hover, None));
    ui.update(|w, cx| w.focus(&chart.read(cx).focus.clone(), cx));
    ui.simulate_keystrokes("right");
    app.read(|cx| assert_eq!(chart.read(cx).hover, Some(0)));
    ui.simulate_keystrokes("right");
    app.read(|cx| assert_eq!(chart.read(cx).hover, Some(1)));
}
