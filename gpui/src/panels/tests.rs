use std::{cell::RefCell, rc::Rc, sync::Arc};

use gewu_app::AppliedView;
use gewu_core::{Dataset, Filter};
use gpui_kit::{
    AppContext, Bounds, Point, TestAppContext, VisualTestContext, WindowBounds, WindowOptions, px,
    size, test::TestWindowExt,
};

use super::{FilterEditor, FilterEvent, Inspector};

pub(super) fn dataset() -> AppliedView {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/workflow.csv");
    AppliedView::new(Arc::new(Dataset::open(path).unwrap()))
}

fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(850.), px(720.)),
        })),
        ..Default::default()
    }
}

fn settle(ui: &mut VisualTestContext) {
    ui.run_until_parked();
    ui.update(|window, cx| window.render_frame(cx));
    ui.run_until_parked();
}

#[test]
fn selecting_one_exact_value_submits_a_filter_and_retains_alternatives_when_reopened() {
    let cx = TestAppContext::single();
    let snapshot = dataset();
    let col = snapshot.dataset().info().columns[0].clone();
    let submitted = Rc::new(RefCell::new(None));
    let events = submitted.clone();
    let (handle, _editor, _subscription) = cx.update(|cx| {
        gpui_kit::init(cx);
        let (handle, editor) = gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(snapshot.clone(), col.clone(), None, w, cx))
        })
        .unwrap();
        let subscription = cx.subscribe(&editor, move |_, event, _| {
            if let FilterEvent::Apply(id, filter) = event {
                *events.borrow_mut() = Some((*id, filter.clone()));
            }
        });
        (handle, editor, subscription)
    });
    let mut ui = VisualTestContext::from_window(handle, &cx);
    settle(&mut ui);
    ui.update(|w, cx| {
        assert_eq!(w.find("Values").role(), Some(gpui_kit::Role::Tab));
        assert_eq!(w.find("Values").selected(), Some(true));
        assert_eq!(w.find("Text").selected(), Some(false));
        assert_eq!(w.find("include-missing").checked(), Some(true));
        assert_eq!(w.find("apply-filter").checked(), None);
        let group = w.find("value-selection").bounds();
        for member in [
            w.find("all-values").bounds(),
            w.find(("facet", 0usize)).bounds(),
            w.find("include-missing").bounds(),
        ] {
            assert!(group.contains(&member.center()));
        }
        assert!(group.bottom() < w.find("apply-filter").bounds().top());
        w.click("no-values", cx);
        assert_eq!(w.find("include-missing").checked(), Some(false));
        w.click(("facet", 0usize), cx);
        w.click("apply-filter", cx);
    });
    settle(&mut ui);
    let applied = submitted.borrow().clone().expect("Apply event");
    let choices = snapshot
        .filtered()
        .filter_choices(col.id)
        .unwrap()
        .values
        .unwrap();
    assert_eq!(
        applied,
        (
            col.id,
            Some(Filter::exact(vec![choices[0].value.clone()], false, false))
        )
    );
    let filtered = snapshot
        .applying(Arc::new(gewu_app::TableQuery {
            filters: vec![gewu_core::ColumnFilter {
                column: col.id,
                filter: applied.1.clone().unwrap(),
            }],
            ..Default::default()
        }))
        .unwrap();
    let (second, _editor) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(filtered, col, applied.1, w, cx))
        })
        .unwrap()
    });
    let mut reopened = VisualTestContext::from_window(second, &cx);
    settle(&mut reopened);
    reopened.update(|w, _| assert!(w.try_find(("facet", 1usize)).is_some()));
}

#[test]
fn inspector_keeps_axes_within_visible_columns_after_a_chart_switch() {
    let cx = TestAppContext::single();
    let snapshot = dataset();
    let columns = snapshot.dataset().info().columns;
    let (handle, inspector) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(options(), cx, |_, cx| {
            cx.new(|cx| Inspector::new(snapshot.clone(), columns.clone(), cx))
        })
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &cx);
    settle(&mut ui);
    ui.update(|w, cx| w.click("Chart", cx));
    settle(&mut ui);
    let numeric = columns.iter().find(|c| c.capabilities.scatter).unwrap();
    ui.update(|w, cx| {
        w.click("Y axis", cx);
        assert!(
            w.try_find(("axis-option", numeric.id.into_raw() as usize))
                .is_some()
        );
        w.click(("axis-option", numeric.id.into_raw() as usize), cx);
    });
    settle(&mut ui);
    ui.update(|_, cx| {
        inspector.update(cx, |p, cx| {
            p.set_context(
                snapshot.clone(),
                columns
                    .iter()
                    .filter(|c| !c.capabilities.scatter)
                    .cloned()
                    .collect(),
                cx,
            )
        });
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        w.click("Y axis", cx);
        assert!(
            w.try_find(("axis-option", numeric.id.into_raw() as usize))
                .is_none()
        );
    });
}

#[test]
fn chart_picker_keeps_duplicate_column_names_distinct_from_the_source_row_axis() {
    use std::io::Write;

    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    write!(file, "Source row,Source row\n11,22\n33,44\n").unwrap();
    let snapshot = AppliedView::new(Arc::new(gewu_core::Dataset::open(file.path()).unwrap()));
    let columns = snapshot.dataset().info().columns;
    let first = ("axis-option", columns[0].id.into_raw() as usize);
    let second = ("axis-option", columns[1].id.into_raw() as usize);
    let app = TestAppContext::single();
    let (handle, _inspector) = app.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(options(), cx, |_, cx| {
            cx.new(|cx| Inspector::new(snapshot, columns, cx))
        })
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &app);
    settle(&mut ui);
    ui.update(|w, cx| w.click("Chart", cx));
    settle(&mut ui);
    ui.update(|w, cx| {
        w.click("X axis", cx);
        assert_eq!(w.find("Chart").role(), Some(gpui_kit::Role::Tab));
        assert_eq!(w.find("Chart").selected(), Some(true));
        assert_eq!(w.find("Column").selected(), Some(false));
        assert_eq!(
            w.find(("points", 2048usize)).role(),
            Some(gpui_kit::Role::RadioButton)
        );
        assert_eq!(w.find(("points", 2048usize)).checked(), Some(true));
        assert_eq!(w.find(("points", 512usize)).checked(), Some(false));
        assert_eq!(w.find("log-y").checked(), Some(false));
        assert_eq!(w.find("X axis").expanded(), Some(true));
        assert_eq!(
            w.find("axis-source").role(),
            Some(gpui_kit::Role::RadioButton)
        );
        assert_eq!(w.find("axis-source").checked(), Some(true));
        assert_eq!(w.find(first).checked(), Some(false));
        assert_eq!(w.find(second).checked(), Some(false));
        assert_eq!(w.find("axis-source").label(), Some("Source row"));
        assert_eq!(w.find(first).label(), Some("Source row"));
        assert_eq!(w.find(second).label(), Some("Source row"));
        w.click(second, cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        w.click("X axis", cx);
        assert_eq!(w.find(second).checked(), Some(true));
        assert_eq!(w.find(first).selected(), Some(false));
        assert_eq!(w.find("axis-source").checked(), Some(false));
        w.click(second, cx);
        assert!(w.try_find("axis-source").is_none());
    });
}

#[test]
fn brushing_range_bins_preserves_integer_bounds_beyond_f64_precision() {
    use std::io::Write;

    use gewu_core::AxisScale;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("precise.csv");
    let mut csv = std::fs::File::create(&path).unwrap();
    writeln!(csv, "value").unwrap();
    for value in 9007199254740992u64..=9007199254741008 {
        writeln!(csv, "{value}").unwrap();
    }
    drop(csv);
    let snapshot = AppliedView::new(Arc::new(Dataset::open(path).unwrap()));
    let column = snapshot.dataset().info().columns[0].clone();
    let distribution = snapshot
        .analyses()
        .filter_distribution(column.id, 40, AxisScale::Linear)
        .unwrap();
    let ranges = &distribution.as_ref().as_ref().unwrap().ranges;
    let lower = ranges[1].min.as_ref().unwrap().value.clone();
    let upper = ranges[3].max.as_ref().unwrap().value.clone();
    let submitted = Rc::new(RefCell::new(None));
    let events = submitted.clone();
    let app = TestAppContext::single();
    let (handle, _editor, _events) = app.update(|cx| {
        gpui_kit::init(cx);
        let (handle, editor) = gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(snapshot, column, None, w, cx))
        })
        .unwrap();
        let sub = cx.subscribe(&editor, move |_, ev, _| {
            if let FilterEvent::Apply(_, filter) = ev {
                *events.borrow_mut() = filter.clone();
            }
        });
        (handle, editor, sub)
    });
    let mut ui = VisualTestContext::from_window(handle, &app);
    settle(&mut ui);
    ui.update(|w, cx| w.click("Range", cx));
    settle(&mut ui);
    ui.update(|w, cx| {
        let plot = w.find(("range-bin", 1usize)).bounds();
        let low = w.find("Minimum").bounds();
        let high = w.find("Maximum").bounds();
        assert!(
            plot.bottom() < low.top(),
            "bounds belong below the histogram"
        );
        assert_eq!(low.top(), high.top());
        assert!(low.right() < high.left());
        w.drag_to(("range-bin", 1usize), ("range-bin", 3usize), cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| w.click("apply-filter", cx));
    settle(&mut ui);
    assert_eq!(
        *submitted.borrow(),
        Some(Filter::range(Some(lower), Some(upper), true))
    );
}

/// The two long values have identical previews. Only their trailing, multiline text differs.
pub(super) fn long_values() -> (AppliedView, String) {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("long-values.csv");
    let mut csv = std::fs::File::create(&path).unwrap();
    writeln!(csv, "value").unwrap();
    let prefix = format!("x{}", "界".repeat(350_000));
    let chosen = format!("{prefix}\nBÉTA[2]");
    for value in [
        format!("{prefix}\nalpha"),
        chosen.clone(),
        "line\r\nbreak".into(),
    ] {
        writeln!(csv, "\"{value}\"").unwrap();
    }
    drop(csv);
    (
        AppliedView::new(Arc::new(Dataset::open(path).unwrap())),
        chosen,
    )
}

#[test]
fn long_equal_previews_search_the_complete_label_and_submit_only_the_exact_selected_value() {
    let app = TestAppContext::single();
    let (snapshot, chosen) = long_values();
    assert_eq!(
        super::value_preview(&chosen).as_ref(),
        format!("x{}…", "界".repeat(31))
    );
    let col = snapshot.dataset().info().columns[0].clone();
    let facet = snapshot.analyses().filter_choices(col.id).unwrap();
    let choices = facet.values.as_ref().unwrap();
    let target = choices.iter().position(|c| c.value == chosen).unwrap();
    let submitted = Rc::new(RefCell::new(None));
    let captured = submitted.clone();
    let (handle, _editor, _events) = app.update(|cx| {
        gpui_kit::init(cx);
        let (handle, editor) = gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(snapshot.clone(), col, None, w, cx))
        })
        .unwrap();
        let sub = cx.subscribe(&editor, move |_, ev, _| {
            if let FilterEvent::Apply(_, filter) = ev {
                *captured.borrow_mut() = filter.clone();
            }
        });
        (handle, editor, sub)
    });
    let mut ui = VisualTestContext::from_window(handle, &app);
    settle(&mut ui);
    let search = ui.update(|w, cx| {
        for (index, choice) in choices.iter().enumerate() {
            let control = w.find(("facet", index));
            let label = control.label().unwrap();
            assert_eq!(
                label,
                format!("{}, 1 rows", super::value_preview(&choice.label))
            );
            assert!(label.len() < 120 && !label.contains(['\r', '\n']));
            assert!(control.bounds().size.height < px(60.));
        }
        w.click("no-values", cx);
        w.find("Search values").bounds().center()
    });
    ui.simulate_click(search, Default::default());
    ui.simulate_input("bÉta[2]");
    settle(&mut ui);
    ui.update(|w, cx| {
        for index in 0..choices.len() {
            assert_eq!(w.try_find(("facet", index)).is_some(), index == target);
        }
        w.click(("facet", target), cx);
        w.click("apply-filter", cx);
    });
    settle(&mut ui);
    assert_eq!(
        *submitted.borrow(),
        Some(Filter::exact(vec![chosen.clone()], false, false))
    );
    for filter in [
        Filter::contains(chosen.clone(), true, false),
        Filter::range(Some(chosen.clone()), Some(chosen.clone()), false),
        Filter::exact(vec![chosen, "early\nnewline".into()], true, true),
    ] {
        let description = super::filter_description(&filter);
        assert!(description.len() < 240 && !description.contains(['\r', '\n']));
        assert!(description.contains('…') && description.contains("missing"));
    }
    ui.update(|w, cx| w.click("reset-filter", cx));
    settle(&mut ui);
    ui.update(|w, _| {
        for i in 0..choices.len() {
            assert!(w.find(("facet", i)).checked().unwrap());
        }
    });
}
