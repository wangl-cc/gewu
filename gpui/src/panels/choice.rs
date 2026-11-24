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
    _activation: Subscription,
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
            let trigger_focus = select.focus_handle(cx);
            let activation = cx.observe_window_activation(
                window,
                move |control: &mut ChoiceControl<T>, window, cx| {
                    if !window.is_window_active() {
                        // The selector exposes its list's focus handle while open and the trigger's
                        // while closed. Route Cancel to the list so it also discards its search
                        // query.
                        let current = control.select.focus_handle(cx);
                        if current != trigger_focus {
                            current.dispatch_action(&base::actions::Cancel, window, cx);
                        }
                    }
                },
            );
            ChoiceControl {
                options: self.options.clone(),
                select,
                _changes: changes,
                _activation: activation,
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
        // The product owns the trigger frame. Select retains the popup, keyboard handling
        // and accessibility relationship between the trigger and the list.
        let frame = super::input_frame(div(), !empty, &select.focus_handle(cx), window, cx);
        let trigger = Select::new(&select)
            .appearance(false)
            .id(self.id)
            .accessibility_label(self.label)
            .placeholder(self.label)
            .search_placeholder("Find column")
            .disabled(empty)
            .small()
            .border_0()
            .h(px(28. - 2. / window.scale_factor()))
            .text_size(px(13.))
            .rounded(px(6.))
            .w_full()
            .menu_max_h(px(220.));
        frame.w_full().child(trigger).into_any_element()
    }
}
