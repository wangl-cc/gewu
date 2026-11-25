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
        assert!(w.try_find("include-missing").is_none());
        assert_eq!(w.find("apply-filter").checked(), None);
        let group = w.find("value-selection").bounds();
        for member in [
            w.find("all-values").bounds(),
            w.find(("facet", 0usize)).bounds(),
        ] {
            assert!(group.contains(&member.center()));
        }
        assert!(group.bottom() < w.find("apply-filter").bounds().top());
        assert_eq!(w.find("all-values").disabled(), Some(true));
        assert_ne!(w.find("no-values").disabled(), Some(true));
        w.click("no-values", cx);
        assert_ne!(w.find("all-values").disabled(), Some(true));
        assert_eq!(w.find("no-values").disabled(), Some(true));
        assert!(w.try_find("include-missing").is_none());
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
        assert_eq!(w.find("Y axis").value(), Some(numeric.name.as_str()));
        w.click("Y axis", cx);
        assert_eq!(w.find("Y axis").expanded(), Some(true));
        w.press("escape", cx);
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
    ui.update(|w, _| {
        assert_eq!(w.find("Y axis").disabled(), Some(true));
        assert_ne!(w.find("Y axis").value(), Some(numeric.name.as_str()));
    });
}

#[test]
fn inspector_bins_expose_the_same_interval_and_count_as_their_hover_labels() {
    let cx = TestAppContext::single();
    let snapshot = dataset();
    let columns = snapshot.dataset().info().columns;
    let (handle, _) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(options(), cx, |_, cx| {
            cx.new(|cx| {
                let mut panel = Inspector::new(snapshot, columns.clone(), cx);
                panel.inspect(columns[1].id, cx);
                panel
            })
        })
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &cx);
    settle(&mut ui);
    ui.update(|w, _| {
        let bin = w.find(("inspector-histogram-bin", 0usize));
        assert_eq!(bin.role(), Some(gpui_kit::Role::Image));
        assert_eq!(bin.label(), Some("≤ 4 · 1 row"));
        assert!(bin.bounds().size.width > px(0.));
    });
}

#[test]
fn chart_picker_keeps_duplicate_column_names_distinct_from_the_source_row_axis() {
    use std::io::Write;

    let mut file = tempfile::Builder::new().suffix(".csv").tempfile().unwrap();
    write!(file, "Source row,Source row\n11,22\n33,44\n").unwrap();
    let snapshot = AppliedView::new(Arc::new(gewu_core::Dataset::open(file.path()).unwrap()));
    let columns = snapshot.dataset().info().columns;
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
    let (plot_bounds, limits_top_idle) = ui.update(|w, _| {
        assert_eq!(w.find("Chart").role(), Some(gpui_kit::Role::Tab));
        assert_eq!(w.find("Chart").selected(), Some(true));
        assert_eq!(w.find("Column").selected(), Some(false));
        assert_eq!(w.find("log-x").disabled(), Some(true));
        assert_eq!(w.find("log-x").checked(), Some(false));
        assert_ne!(w.find("log-y").disabled(), Some(true));
        assert_eq!(w.find("log-y").checked(), Some(false));
        assert_eq!(w.find("scatter-limits").value(), Some("2,048"));
        assert!(w.try_find(("points", 2048usize)).is_none());
        assert!(w.try_find("scatter-readout").is_none());
        (
            w.find("scatter-plot").bounds(),
            w.find("scatter-limits").bounds().top(),
        )
    });
    let data = crate::panels::scatter_data_bounds(plot_bounds);
    ui.simulate_mouse_move(
        gpui_kit::point(data.left(), data.bottom()),
        None,
        gpui_kit::Modifiers::default(),
    );
    settle(&mut ui);
    ui.update(|w, cx| {
        let readout = w.find("scatter-readout").bounds();
        let limits = w.find("scatter-limits").bounds();
        assert_eq!(limits.top(), limits_top_idle);
        assert!(limits.bottom() <= readout.top());
        w.click("scatter-limits", cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        assert_eq!(
            w.find(("points", 2048usize)).role(),
            Some(gpui_kit::Role::RadioButton)
        );
        assert_eq!(w.find(("points", 2048usize)).checked(), Some(true));
        assert_eq!(w.find(("points", 512usize)).checked(), Some(false));
        w.press("escape", cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        assert!(w.try_find(("points", 2048usize)).is_none());
        w.click("X axis", cx);
        assert_eq!(w.find("X axis").expanded(), Some(true));
        assert_eq!(w.find("X axis").role(), Some(gpui_kit::Role::ComboBox));
        assert_eq!(w.find("X axis").value(), Some("Source row"));
        w.press("down", cx);
        w.press("down", cx);
        w.press("enter", cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        assert_eq!(w.find("X axis").value(), Some("Source row · column 2"));
        assert_ne!(w.find("log-x").disabled(), Some(true));
        w.click("X axis", cx);
        w.press("up", cx);
        w.press("enter", cx);
    });
    settle(&mut ui);
    ui.update(|w, _| {
        assert_eq!(w.find("X axis").value(), Some("Source row · column 1"));
        assert_eq!(w.find("X axis").expanded(), Some(false));
    });
}

#[test]
fn missing_policy_controls_hide_without_nulls_and_appear_when_nullable() {
    let cx = TestAppContext::single();
    let complete = dataset();
    let group_col = complete.dataset().info().columns[0].clone();
    let score = complete.dataset().info().columns[1].clone();
    let (handle, _) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(complete.clone(), score.clone(), None, w, cx))
        })
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &cx);
    settle(&mut ui);
    ui.update(|w, cx| w.click("Range", cx));
    settle(&mut ui);
    ui.update(|w, _| {
        assert!(w.try_find("exclude-missing").is_none());
        assert!(w.try_find("only-missing").is_none());
    });
    let (text_handle, _) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(complete.clone(), group_col, None, w, cx))
        })
        .unwrap()
    });
    let mut text_ui = VisualTestContext::from_window(text_handle, &cx);
    settle(&mut text_ui);
    text_ui.update(|w, cx| w.click("Text", cx));
    settle(&mut text_ui);
    text_ui.update(|w, _| assert!(w.try_find("only-missing").is_none()));

    let (complete_filtered, _) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |w, cx| {
            let mut query = gewu_app::TableQuery::default();
            query.filters.push(gewu_core::ColumnFilter {
                column: score.id,
                filter: Filter::range(Some("2".into()), None, false),
            });
            let filtered = complete.applying(Arc::new(query)).unwrap();
            cx.new(|cx| {
                FilterEditor::new(
                    filtered,
                    score,
                    Some(Filter::range(Some("2".into()), None, false)),
                    w,
                    cx,
                )
            })
        })
        .unwrap()
    });
    let mut filtered_ui = VisualTestContext::from_window(complete_filtered, &cx);
    settle(&mut filtered_ui);
    filtered_ui.update(|w, cx| w.click("Range", cx));
    settle(&mut filtered_ui);
    filtered_ui.update(|w, _| {
        assert!(w.try_find("exclude-missing").is_some());
        assert!(w.try_find("only-missing").is_some());
    });

    let dir = tempfile::tempdir().unwrap();
    let nullable_path = dir.path().join("nullable.csv");
    std::fs::write(&nullable_path, "label,amount\na,10\n,20\nb,\n").unwrap();
    let nullable = AppliedView::new(Arc::new(Dataset::open(nullable_path).unwrap()));
    let label_col = nullable.dataset().info().columns[0].clone();
    let amount_col = nullable.dataset().info().columns[1].clone();
    let (h2, _) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(nullable.clone(), label_col, None, w, cx))
        })
        .unwrap()
    });
    let mut ui2 = VisualTestContext::from_window(h2, &cx);
    settle(&mut ui2);
    ui2.update(|w, cx| {
        assert!(w.try_find("include-missing").is_some());
        assert!(
            w.find("value-selection")
                .bounds()
                .contains(&w.find("include-missing").bounds().center())
        );
        w.click("Text", cx);
    });
    settle(&mut ui2);
    ui2.update(|w, _| {
        assert!(w.try_find("exclude-missing").is_some());
        assert!(w.try_find("only-missing").is_some());
    });

    let (h3, _) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(nullable, amount_col, None, w, cx))
        })
        .unwrap()
    });
    let mut ui3 = VisualTestContext::from_window(h3, &cx);
    settle(&mut ui3);
    ui3.update(|w, cx| w.click("Range", cx));
    settle(&mut ui3);
    ui3.update(|w, _| assert!(w.try_find("only-missing").is_some()));
}

#[test]
fn column_inspector_aligns_picker_full_width_and_omits_redundant_storage() {
    let cx = TestAppContext::single();
    let snapshot = dataset();
    let columns = snapshot.dataset().info().columns;
    let (handle, _) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(options(), cx, |_, cx| {
            cx.new(|cx| {
                let mut panel = Inspector::new(snapshot, columns.clone(), cx);
                panel.inspect(columns[1].id, cx);
                panel
            })
        })
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &cx);
    settle(&mut ui);
    ui.update(|w, cx| {
        let column_tab = w.find("Column").bounds();
        let chart_tab = w.find("Chart").bounds();
        let picker = w.find("Inspect column").bounds();
        assert!(picker.left() <= column_tab.left());
        assert!(picker.right() >= chart_tab.right());
        let texts = gpui_kit::base::test_support::texts(w);
        assert!(
            !texts.iter().any(|t| t.text() == "Storage"),
            "int64 column should not repeat Storage: Int64"
        );
        assert!(w.try_find(("bins", 40usize)).is_none());
        w.click("histogram-options", cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        assert_eq!(
            w.find(("bins", 40usize)).role(),
            Some(gpui_kit::Role::RadioButton)
        );
        assert_eq!(w.find(("bins", 40usize)).checked(), Some(true));
        assert_eq!(
            w.find("inspector-log-counts").role(),
            Some(gpui_kit::Role::CheckBox)
        );
        w.press("escape", cx);
    });
    settle(&mut ui);
    ui.update(|w, _| assert!(w.try_find(("bins", 40usize)).is_none()));
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
        assert!(w.try_find("filter-log-counts").is_none());
        w.click("range-display-options", cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        assert_eq!(
            w.find("filter-log-counts").role(),
            Some(gpui_kit::Role::CheckBox)
        );
        w.click("range-display-options", cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        assert!(w.try_find("filter-log-counts").is_none());
        let plot = w.find(("range-bin", 1usize)).bounds();
        let low = w.find("Minimum").bounds();
        let high = w.find("Maximum").bounds();
        assert!(
            plot.bottom() < low.top(),
            "bounds belong below the histogram"
        );
        assert_eq!(low.size.height, gpui_kit::px(28.));
        assert_eq!(low.size.height, high.size.height);
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

#[test]
fn dragging_range_handles_adjusts_bounds_independently() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("handles.csv");
    let rows = (0..=10).map(|i| format!("{i}\n")).collect::<String>();
    std::fs::write(&path, format!("value\n{rows}")).unwrap();
    let snapshot = AppliedView::new(Arc::new(Dataset::open(path).unwrap()));
    let column = snapshot.dataset().info().columns[0].clone();
    let distribution = snapshot
        .analyses()
        .filter_distribution(column.id, 40, gewu_core::AxisScale::Linear)
        .unwrap();
    let ranges = &distribution.as_ref().as_ref().unwrap().ranges;
    let last = ranges.len() - 1;
    let lower = ranges[2].min.as_ref().unwrap().value.clone();
    let upper = ranges[4].max.as_ref().unwrap().value.clone();
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
        let min_handle = w.find("range-handle-min").bounds();
        let max_handle = w.find("range-handle-max").bounds();
        assert!(min_handle.right() <= w.find(("range-bin", 0usize)).bounds().left());
        assert!(max_handle.left() >= w.find(("range-bin", last)).bounds().right());
        w.drag_to("range-handle-min", ("range-bin", 2usize), cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        w.drag_to("range-handle-max", ("range-bin", 4usize), cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| w.click("apply-filter", cx));
    settle(&mut ui);
    assert_eq!(
        *submitted.borrow(),
        Some(Filter::range(Some(lower), Some(upper), true))
    );
}

#[test]
fn brushed_float_preview_preserves_exact_bounds_and_yields_to_manual_edits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("float.csv");
    std::fs::write(&path, "temperature\n10.2\n12.7\n19.2\n").unwrap();
    let snapshot = AppliedView::new(Arc::new(Dataset::open(path).unwrap()));
    let column = snapshot.dataset().info().columns[0].clone();
    let distribution = snapshot
        .analyses()
        .filter_distribution(column.id, 40, gewu_core::AxisScale::Linear)
        .unwrap();
    let ranges = &distribution.as_ref().as_ref().unwrap().ranges;
    let index = ranges
        .iter()
        .position(|bin| {
            bin.min.is_some() && bin.max.as_ref().is_some_and(|bound| bound.value.len() > 14)
        })
        .expect("a bin with a long inclusive maximum");
    let lower = ranges[index].min.as_ref().unwrap().value.clone();
    let upper = ranges[index].max.as_ref().unwrap().value.clone();

    let submitted = Rc::new(RefCell::new(None));
    let events = submitted.clone();
    let app = TestAppContext::single();
    let (handle, _editor, _subscription) = app.update(|cx| {
        gpui_kit::init(cx);
        let (handle, editor) = gpui_kit::open_window(options(), cx, |w, cx| {
            cx.new(|cx| FilterEditor::new(snapshot, column, None, w, cx))
        })
        .unwrap();
        let subscription = cx.subscribe(&editor, move |_, event, _| {
            if let FilterEvent::Apply(_, filter) = event {
                *events.borrow_mut() = filter.clone();
            }
        });
        (handle, editor, subscription)
    });
    let mut ui = VisualTestContext::from_window(handle, &app);
    settle(&mut ui);
    ui.update(|w, cx| w.click("Range", cx));
    settle(&mut ui);
    ui.update(|w, cx| w.click(("range-bin", index), cx));
    settle(&mut ui);
    ui.update(|w, cx| {
        assert!(w.try_find(("brush-preview", 1usize)).is_some());
        w.click("apply-filter", cx);
    });
    assert_eq!(
        *submitted.borrow(),
        Some(Filter::range(Some(lower.clone()), Some(upper), true))
    );
    ui.update(|w, cx| w.click(("brush-preview", 1usize), cx));
    settle(&mut ui);
    ui.update(|w, cx| {
        assert!(w.try_find(("brush-preview", 1usize)).is_none());
        w.press("cmd-a", cx);
        w.input("19.212345678901234", cx);
        w.click("Minimum", cx);
    });
    settle(&mut ui);
    ui.update(|w, cx| {
        assert!(w.try_find(("brush-preview", 1usize)).is_none());
        w.click("apply-filter", cx);
    });
    assert_eq!(
        *submitted.borrow(),
        Some(Filter::range(
            Some(lower),
            Some("19.212345678901234".into()),
            true
        ))
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
                format!("{}, 1 row", super::value_preview(&choice.label))
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
