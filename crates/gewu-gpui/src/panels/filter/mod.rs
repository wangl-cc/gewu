//! Per-column editor. All counts and range bounds come from this column's facet.
use std::sync::Arc;

use gewu_app::AppliedView;
use gewu_core::{
    AxisScale, ColumnId, ColumnInfo, Filter, FilterChoices, RangeHistogram, TextMatch,
};
use gpui_kit::{
    base::{
        self,
        input::{Input, InputEvent, InputState},
    },
    *,
};

use super::{
    charts,
    draft::{Draft, MissingPolicy, Mode},
    row, section,
};

const PREVIEW_BINS: u32 = 40;

pub(crate) enum FilterEvent {
    Apply(ColumnId, Option<Filter>),
    Close,
}

#[derive(Copy, Clone)]
enum Brush {
    Extend {
        anchor: usize,
    },
    Pan {
        start: usize,
        lower: usize,
        upper: usize,
    },
}

impl Brush {
    fn span(self, at: usize, n: usize) -> (usize, usize) {
        match self {
            Self::Extend { anchor } => (anchor.min(at), anchor.max(at)),
            Self::Pan {
                start,
                lower,
                upper,
            } => {
                let shift = (at as isize - start as isize)
                    .clamp(-(lower as isize), (n - 1 - upper) as isize);
                (
                    lower.saturating_add_signed(shift),
                    upper.saturating_add_signed(shift),
                )
            }
        }
    }
}

mod choices;
use choices::ChoiceRow;

pub(crate) struct FilterEditor {
    snapshot: AppliedView,
    column: ColumnInfo,
    draft: Draft,
    rows: Vec<ChoiceRow>,
    matches: Vec<usize>,
    search_task: Option<Task<()>>,
    preview: Option<Arc<Option<RangeHistogram>>>,
    drag: Option<Brush>,
    scale: AxisScale,
    log_counts: bool,
    display_options: bool,
    search: Entity<InputState>,
    text: Entity<InputState>,
    low: Entity<InputState>,
    high: Entity<InputState>,
    brushed_bounds: [Option<String>; 2],
    _bound_changes: [Subscription; 2],
    _input: Subscription,
    load: Option<Task<()>>,
    error: Option<gewu_core::Error>,
}

impl EventEmitter<FilterEvent> for FilterEditor {}

impl FilterEditor {
    pub(crate) fn new(
        snapshot: AppliedView,
        column: ColumnInfo,
        applied: Option<Filter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = Draft::from_filter(applied.as_ref());
        let mut field = |placeholder: &'static str, value: &str| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value.to_owned())
            })
        };
        let search = field("Search values", "");
        let text = field("Text to match", &draft.search);
        let low = field("No minimum", &draft.low);
        let high = field("No maximum", &draft.high);
        let bound_changes = [(0, &low), (1, &high)].map(|(index, input)| {
            cx.subscribe(
                input,
                move |editor: &mut Self, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change)
                        && editor.brushed_bounds[index].as_deref()
                            != Some(input.read(cx).value().as_ref())
                    {
                        editor.brushed_bounds[index] = None;
                    }
                    if matches!(
                        event,
                        InputEvent::Change | InputEvent::Focus | InputEvent::Blur
                    ) {
                        cx.notify();
                    }
                },
            )
        });
        let subscription = cx.subscribe(&search, |editor: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                editor.search_choices(cx);
                cx.notify();
            }
        });
        let mut this = Self {
            snapshot,
            column,
            draft,
            rows: Vec::new(),
            matches: Vec::new(),
            search_task: None,
            preview: None,
            scale: AxisScale::Linear,
            drag: None,
            log_counts: false,
            display_options: false,
            search,
            text,
            low,
            high,
            brushed_bounds: [None, None],
            _bound_changes: bound_changes,
            _input: subscription,
            load: None,
            error: None,
        };
        this.load(cx);
        this
    }

    pub(crate) fn set_snapshot(&mut self, snapshot: AppliedView, cx: &mut Context<Self>) {
        let changed = !Arc::ptr_eq(snapshot.filtered(), self.snapshot.filtered());
        self.snapshot = snapshot;
        if changed {
            self.draft.set_facet(None);
            self.rows.clear();
            self.matches.clear();
            self.search_task = None;
            self.load(cx);
            cx.notify();
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.snapshot.clone();
        let id = self.column.id;
        let capabilities = self.column.capabilities;
        let scale = self.scale;
        let mode = self.draft.mode;
        let keep_exact = self.draft.has_exact_condition();
        self.preview = None;
        self.drag = None;
        self.error = None;
        self.load = Some(cx.spawn(async move |this, cx| {
            let result: gewu_core::Result<_> = cx
                .background_spawn(async move {
                    let cache = snapshot.analyses();
                    let facet = cache.filter_choices(id)?;
                    let modes = Draft::modes(capabilities, facet.values.is_some() || keep_exact);
                    let mode = if modes.contains(&mode) {
                        mode
                    } else {
                        modes[0]
                    };
                    let preview = if mode == Mode::Range {
                        Some(cache.filter_distribution(id, PREVIEW_BINS, scale)?)
                    } else {
                        None
                    };
                    Ok((facet, modes, preview))
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                match result {
                    Ok((facet, modes, preview)) => {
                        if !modes.contains(&editor.draft.mode) {
                            editor.draft.mode = modes[0];
                        }
                        editor.install_choices(facet, cx);
                        editor.preview = preview;
                    }
                    Err(error) => editor.error = Some(error),
                }
                cx.notify();
            });
        }));
    }

    fn selected_bins(&self, cx: &App) -> Option<(usize, usize)> {
        let h = self.preview.as_deref()?.as_ref()?;
        let low = self.low.read(cx).value();
        let high = self.high.read(cx).value();
        if low.is_empty() && high.is_empty() {
            return None;
        }
        let a = if low.is_empty() {
            0
        } else {
            h.ranges
                .iter()
                .position(|b| b.min.as_ref().is_some_and(|v| v.value == low.as_ref()))?
        };
        let b = if high.is_empty() {
            h.ranges.len().checked_sub(1)?
        } else {
            h.ranges
                .iter()
                .rposition(|b| b.max.as_ref().is_some_and(|v| v.value == high.as_ref()))?
        };
        (a <= b).then_some((a, b))
    }

    fn start_brush(
        &mut self,
        at: usize,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = Some(match self.selected_bins(cx) {
            Some((a, b)) if extend => Brush::Extend {
                anchor: if at.abs_diff(a) < at.abs_diff(b) {
                    b
                } else {
                    a
                },
            },
            Some((a, b)) if a != b && at == a => Brush::Extend { anchor: b },
            Some((a, b)) if a != b && at == b => Brush::Extend { anchor: a },
            Some((a, b)) if a < at && at < b => Brush::Pan {
                start: at,
                lower: a,
                upper: b,
            },
            _ => Brush::Extend { anchor: at },
        });
        self.brush(at, window, cx);
    }

    pub(super) fn start_handle_drag(
        &mut self,
        is_end: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self
            .preview
            .as_deref()
            .and_then(Option::as_ref)
            .map_or(0, |h| h.ranges.len());
        let Some(last_index) = n.checked_sub(1) else {
            return;
        };
        let (first, last) = self.selected_bins(cx).unwrap_or((0, last_index));
        let (anchor, at) = if is_end { (first, last) } else { (last, first) };
        self.drag = Some(Brush::Extend { anchor });
        self.brush(at, window, cx);
    }

    fn brush(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(drag), Some(histogram)) =
            (self.drag, self.preview.as_deref().and_then(Option::as_ref))
        else {
            return;
        };
        let (a, b) = drag.span(index, histogram.ranges.len());
        let (Some(first), Some(last)) = (histogram.ranges.get(a), histogram.ranges.get(b)) else {
            return;
        };
        // Only the exact Core text goes back to the query, including Decimal256 and nanoseconds.
        let low = first
            .min
            .as_ref()
            .map(|b| b.value.clone())
            .unwrap_or_default();
        let high = last
            .max
            .as_ref()
            .map(|b| b.value.clone())
            .unwrap_or_default();
        if self.low.read(cx).value().as_ref() == low && self.high.read(cx).value().as_ref() == high
        {
            return;
        }
        self.brushed_bounds = [Some(low.clone()), Some(high.clone())];
        self.low
            .update(cx, |input, cx| input.set_value(low, window, cx));
        self.high
            .update(cx, |input, cx| input.set_value(high, window, cx));
        cx.notify();
    }

    fn select_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.draft.mode = mode;
        if mode == Mode::Range && self.preview.is_none() {
            self.load(cx);
        }
        cx.notify();
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.reset();
        let modes = Draft::modes(
            self.column.capabilities,
            self.draft.facet().is_some_and(|f| f.values.is_some()),
        );
        self.drag = None;
        for input in [&self.search, &self.text, &self.low, &self.high] {
            input.update(cx, |field, cx| field.set_value("", window, cx));
        }
        self.search_choices(cx);
        self.select_mode(modes[0], cx);
    }
}

mod range;
mod render;
