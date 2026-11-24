use super::*;

fn facet(values: &[&str]) -> Arc<FilterChoices> {
    Arc::new(FilterChoices {
        values: Some(
            values
                .iter()
                .map(|s| gewu_core::ValueChoice {
                    label: (*s).into(),
                    value: (*s).into(),
                    count: 1,
                })
                .collect(),
        ),
        null_count: 0,
    })
}

#[test]
fn deselect_all_includes_missing_and_retains_distinct_utf8_literals() {
    let mut draft = Draft {
        mode: Mode::Values,
        ..Default::default()
    };
    draft.select_all(false);
    let facet = facet(&["", "é", "e\u{301}"]);
    draft.set_facet(Some(facet.clone()));
    for index in 0..3 {
        draft.toggle(&facet, index);
    }
    let Some(Filter::Condition {
        condition: FilterCondition::Exact { values, exclude },
        include_nulls,
    }) = draft.filter()
    else {
        panic!("expected exact condition")
    };
    assert_eq!(values, ["", "e\u{301}", "é"]);
    assert!(!exclude && !include_nulls);
    draft.select_all(true);
    assert_eq!(draft.filter(), None);
}

#[test]
fn range_retains_native_precision_and_drafts_survive_switching_modes() {
    let applied = Filter::range(Some("9007199254740993".into()), None, false);
    let mut draft = Draft::from_filter(Some(&applied));
    draft.mode = Mode::Text;
    draft.search = "town".into();
    assert!(matches!(
        draft.filter(),
        Some(Filter::Condition {
            condition: FilterCondition::Text { .. },
            ..
        })
    ));
    draft.mode = Mode::Range;
    assert_eq!(draft.filter(), Some(applied));
}

#[test]
fn choosing_a_value_after_only_missing_keeps_missing_and_the_chosen_value() {
    let mut draft = Draft::from_filter(Some(&Filter::OnlyNulls));
    let facet = facet(&["v"]);
    draft.set_facet(Some(facet.clone()));
    draft.toggle(&facet, 0);
    assert_eq!(
        draft.filter(),
        Some(Filter::exact(vec!["v".into()], false, true))
    );
}

#[test]
fn replacing_the_facet_rebinds_checks_and_preserves_off_list_selections() {
    let applied = Filter::exact(vec!["a".into(), "hidden".into()], false, true);
    let mut draft = Draft::from_filter(Some(&applied));
    let first = facet(&["a", "b"]);
    draft.set_facet(Some(first.clone()));
    assert!(draft.value_checked(0));
    assert!(!draft.value_checked(1));

    let reordered = facet(&["b", "a"]);
    draft.set_facet(Some(reordered.clone()));
    draft.toggle(&first, 0);
    assert!(!draft.value_checked(0));
    assert!(draft.value_checked(1));
    assert_eq!(draft.filter(), Some(applied));

    draft.toggle(&reordered, 1);
    assert!(!draft.value_checked(1));
    assert_eq!(
        draft.filter(),
        Some(Filter::exact(vec!["hidden".into()], false, true))
    );
}
