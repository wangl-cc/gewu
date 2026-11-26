use std::sync::Arc;

use gewu_core::{AxisScale, ScatterAxisKind, ScatterPoint, ScatterResult};
use gpui_kit::{
    AppContext, Bounds, Modifiers, Point, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, point, px, size, test::TestWindowExt,
};

use super::{Plot, ScatterChart, TICK_LINE_HEIGHT, data_bounds, plot_bounds, tick_label_top};

#[test]
fn corner_tick_labels_have_clearance_and_fit_in_the_canvas() {
    let canvas = Bounds::new(point(px(30.), px(60.)), size(px(264.), px(198.)));
    let baseline = plot_bounds(canvas).bottom();
    let y_label_bottom = tick_label_top(baseline, true) + TICK_LINE_HEIGHT;
    let x_label_top = tick_label_top(baseline, false);

    assert!(x_label_top - y_label_bottom >= px(4.));
    assert!(x_label_top + TICK_LINE_HEIGHT <= canvas.bottom());
}

#[test]
fn extreme_points_receive_inset_padding_from_plot_edges() {
    let canvas = Bounds::new(point(px(0.), px(0.)), size(px(276.), px(207.)));
    let drawing = plot_bounds(canvas);
    let data = data_bounds(drawing);
    let min_pt = Plot::point(data, (0., 0.));
    let max_pt = Plot::point(data, (1., 1.));
    assert!(min_pt.x - drawing.left() >= px(6.));
    assert!(drawing.right() - max_pt.x >= px(6.));
    assert!(drawing.bottom() - min_pt.y >= px(6.));
    assert!(max_pt.y - drawing.top() >= px(6.));
}

#[test]
fn hovering_the_plotted_position_reads_the_original_row_and_exact_text() {
    let sample = Arc::new(ScatterResult {
        x_axis_kind: ScatterAxisKind::Number,
        y_axis_kind: ScatterAxisKind::Number,
        eligible_rows: 2,
        non_positive_rows: 0,
        points: vec![
            ScatterPoint {
                source_row: gewu_core::SourceRow(4),
                x: 4.,
                y: 2.,
                x_label: "4".into(),
                y_label: "2.000000000000001".into(),
            },
            ScatterPoint {
                source_row: gewu_core::SourceRow(8),
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
                        true,
                        cx,
                    )
                })
            },
        )
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &app);
    ui.update(|w, cx| w.render_frame(cx));
    let bounds = ui.update(|w, _| {
        let plot = w.find("scatter-plot");
        assert_eq!(plot.label(), Some("Scatter plot, 2 points"));
        plot.bounds()
    });
    let plot = data_bounds(plot_bounds(bounds));
    let first = point(plot.left(), plot.bottom());
    ui.simulate_mouse_move(first, None, Modifiers::default());
    ui.run_until_parked();
    app.read(|cx| {
        let c = chart.read(cx);
        assert_eq!(c.hover, Some(0));
        assert_eq!(c.sample.points[0].source_row, gewu_core::SourceRow(4));
        assert_eq!(c.sample.points[0].y_label, "2.000000000000001");
    });
    ui.update(|w, cx| w.drag(first, point(plot.right(), plot.top()), cx));
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
