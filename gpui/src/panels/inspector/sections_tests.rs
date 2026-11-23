use std::sync::Arc;

use gewu_core::{AxisScale, ColumnInfo};

use super::{ColumnResults, InspectorSettings, Section, SectionData};

#[test]
fn expanded_sections_follow_capabilities_and_keep_unaffected_cached_analysis() {
    let snapshot = crate::panels::tests::dataset();
    let schema = snapshot.dataset().info().columns;
    let numeric = &schema[1];
    let text = &schema[0];
    let mut settings = InspectorSettings::default();
    let selected = |col: &ColumnInfo, s: InspectorSettings| {
        Section::available(col)
            .filter(|k| s.expanded(*k, col))
            .collect::<Vec<_>>()
    };
    assert_eq!(selected(numeric, settings), [
        Section::Overview,
        Section::Statistics,
        Section::Distribution
    ]);
    assert_eq!(selected(text, settings), [
        Section::Overview,
        Section::Values
    ]);
    let snapshots = |s| ColumnResults::load(&snapshot, numeric, s, AxisScale::Linear);
    let result = snapshots(settings);
    let stats = |r: &ColumnResults| {
        r.sections
            .iter()
            .find_map(|(_, data)| {
                if let Some(Ok(SectionData::Statistics(s))) = data {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .unwrap()
    };
    let original = stats(&result);
    settings.bins = 2;
    let changed = snapshots(settings);
    assert!(Arc::ptr_eq(&stats(&changed), &original));
    settings.expansion[Section::Statistics as usize] = Some(false);
    settings.expansion[Section::Values as usize] = Some(true);
    assert_eq!(selected(numeric, settings), [
        Section::Overview,
        Section::Distribution,
        Section::Values
    ]);
    assert!(
        snapshots(settings)
            .sections
            .iter()
            .any(|(s, d)| *s == Section::Statistics && d.is_none())
    );
}

#[test]
fn changing_rows_column_and_bin_count_only_displays_the_final_analysis() {
    use gewu_app::TableQuery;
    use gewu_core::{ColumnFilter, Filter};
    use gpui_kit::{AppContext, TestAppContext};

    use super::super::{Inspector, Page, Showing};

    let original = crate::panels::tests::dataset();
    let columns = original.dataset().info().columns;
    let score = columns[1].id;
    let filtered = original
        .applying(Arc::new(TableQuery {
            filters: vec![ColumnFilter {
                column: columns[0].id,
                filter: Filter::exact(vec!["keep".into()], false, false),
            }],
            ..Default::default()
        }))
        .unwrap();
    let app = TestAppContext::single();
    let inspector = app.update(|cx| cx.new(|cx| Inspector::new(original, columns.clone(), cx)));
    app.update(|cx| inspector.update(cx, |panel, cx| panel.set_page(Page::Chart, cx)));
    app.run_until_parked();
    let old_chart = app.read(|cx| {
        let panel = inspector.read(cx);
        let Showing::Chart(chart) = &panel.showing else {
            panic!("expected chart")
        };
        assert_eq!(panel.plotted_column(chart.entity_id()), Some(score));
        chart.entity_id()
    });
    app.update(|cx| {
        inspector.update(cx, |panel, cx| {
            panel.inspect(score, cx);
            panel.settings.page = Page::Chart;
            panel.load(cx);
            panel.inspect(columns[0].id, cx);
            panel.set_context(filtered.clone(), columns, cx);
            panel.settings.bins = 4;
            panel.inspect(score, cx);
        })
    });
    app.run_until_parked();
    app.read(|cx| {
        assert_eq!(inspector.read(cx).plotted_column(old_chart), None);
        let Showing::Column(result) = &inspector.read(cx).showing else {
            panic!("expected column analysis")
        };
        let histogram = result
            .sections
            .iter()
            .find_map(|(_, data)| {
                if let Some(Ok(SectionData::Distribution(histogram))) = data {
                    Some(histogram)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(histogram.histogram.counts.iter().sum::<u64>(), 3);
        let expected = filtered
            .analyses()
            .distribution_with_ranges(score, 4, AxisScale::Linear)
            .unwrap();
        assert!(Arc::ptr_eq(histogram, &expected));
    });
}

#[test]
fn common_values_bound_each_label_and_preserve_frequencies_and_full_cached_values() {
    let (snapshot, chosen) = crate::panels::tests::long_values();
    let col = &snapshot.dataset().info().columns[0];
    let data = ColumnResults::load(
        &snapshot,
        col,
        InspectorSettings::default(),
        AxisScale::Linear,
    );
    let rows = data
        .sections
        .iter()
        .find_map(|(_, d)| {
            if let Some(Ok(SectionData::Values { rows, .. })) = d {
                Some(rows)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter().all(|(label, count)| label.len() < 100
            && !label.contains(['\r', '\n'])
            && *count == 1)
    );
    let preview = crate::panels::value_preview(&chosen);
    assert_eq!(
        rows.iter().filter(|(label, _)| *label == preview).count(),
        2
    );
    assert!(
        snapshot
            .analyses()
            .values(col.id)
            .unwrap()
            .values
            .iter()
            .any(|v| v.label == chosen)
    );
}
