use super::*;

#[test]
fn deselect_all_includes_missing_and_retains_distinct_utf8_literals() {
    let mut draft = Draft {
        mode: Mode::Values,
        ..Default::default()
    };
    draft.select_all(false);
    for value in ["", "é", "e\u{301}"] {
        draft.toggle(value.into());
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
    draft.toggle("v".into());
    assert_eq!(
        draft.filter(),
        Some(Filter::exact(vec!["v".into()], false, true))
    );
}
