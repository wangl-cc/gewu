//! Adapt the toolkit's searchable selector to choices derived from the current document.
use gewu_core::{ColumnId, ColumnInfo};
use gpui_kit::{
    component::{
        IndexPath, Sizable,
        searchable_list::{SearchableListItem, SearchableVec},
        select::{Select, SelectEvent, SelectState},
    },
    *,
};

#[derive(Clone, PartialEq)]
pub(crate) struct Choice<T> {
    pub value: T,
    pub label: SharedString,
}

/// Duplicate names include the source column position, even when only one is selectable.
pub(crate) fn column_labels<'a>(
    columns: impl Iterator<Item = &'a ColumnInfo> + Clone,
    reserved: Option<&'static str>,
) -> impl Iterator<Item = (ColumnId, SharedString)> {
    let mut seen: std::collections::HashSet<_> = reserved.into_iter().collect();
    let duplicates: std::collections::HashSet<_> = columns
        .clone()
        .filter_map(|c| (!seen.insert(c.name.as_str())).then_some(c.name.as_str()))
        .collect();
    columns.map(move |c| {
        let label = if duplicates.contains(c.name.as_str()) {
            format!("{} · column {}", c.name, u64::from(c.id.into_raw()) + 1).into()
        } else {
            c.name.clone().into()
        };
        (c.id, label)
    })
}

impl<T: Clone + PartialEq> SearchableListItem for Choice<T> {
    type Value = T;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id(self.label.clone())
            .role(Role::Label)
            .aria_label(self.label.clone())
            .test_support()
            .truncate()
            .child(self.label.clone())
    }

    fn value(&self) -> &T {
        &self.value
    }
}

pub(crate) struct ChoiceField<T> {
    pub id: ElementId,
    pub label: &'static str,
    pub options: Vec<Choice<T>>,
    pub selected: Option<T>,
    pub searchable: bool,
}

struct ChoiceControl<T: Clone + PartialEq + 'static> {
    options: Vec<Choice<T>>,
    select: Entity<SelectState<SearchableVec<Choice<T>>>>,
    _changes: Subscription,
}

impl<T: Clone + PartialEq + 'static> ChoiceField<T> {
    pub fn element(
        self,
        window: &mut Window,
        cx: &mut App,
        choose: impl Fn(&T, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let empty = self.options.is_empty();
        // Keep the same trigger and keyboard focus when an option is committed, including when
        // choosing a sort key changes the options still available to its neighbors.
        let control = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            let index = self
                .options
                .iter()
                .position(|c| Some(&c.value) == self.selected.as_ref());
            let select = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::from(self.options.clone()),
                    index.map(IndexPath::new),
                    window,
                    cx,
                )
                .searchable(self.searchable)
            });
            let changes = cx.subscribe_in(&select, window, move |_, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    choose(value, window, cx);
                }
            });
            ChoiceControl {
                options: self.options.clone(),
                select,
                _changes: changes,
            }
        });
        let select = control.read(cx).select.clone();
        if control.read(cx).options != self.options {
            let items = SearchableVec::from(self.options.clone());
            select.update(cx, |s, cx| s.set_items(items, window, cx));
            control.update(cx, |control, _| control.options = self.options);
        }
        if select.read(cx).selected_value() != self.selected.as_ref() {
            select.update(cx, |s, cx| match &self.selected {
                Some(value) => s.set_selected_value(value, window, cx),
                None => s.set_selected_index(None, window, cx),
            });
        }
        Select::new(&select)
            .id(self.id)
            .accessibility_label(self.label)
            .placeholder(self.label)
            .search_placeholder("Find column")
            .disabled(empty)
            .small()
            .w_full()
            .menu_max_h(px(220.))
            .into_any_element()
    }
}
