//! Pointer, wheel and keyboard input for the grid. Each handler accepts the frame that was
//! drawn, hands the input to the [`GridModel`](super::grid::GridModel) and carries out the
//! work it returns.
use super::{
    grid::{DragUpdate, Geometry, GridEffect},
    *,
};

impl TableView {
    pub(super) fn show_scrollbars(&self, cx: &App) -> bool {
        !cx.should_auto_hide_scrollbars() || self.scrollbar_timer.is_some()
    }

    /// Run a model operation against the committed geometry and report a changed selection.
    fn with_grid<R>(
        &mut self,
        cx: &mut Context<Self>,
        operation: impl FnOnce(&mut GridModel, &Geometry) -> R,
    ) -> R {
        let before = self.grid.selection();
        let geometry = Geometry {
            layout: &self.layout,
            columns: &self.data.columns,
            auto_hide_scrollbars: cx.should_auto_hide_scrollbars(),
        };
        let result = operation(&mut self.grid, &geometry);
        if self.grid.selection() != before {
            cx.emit(TableEvent::SelectionChanged);
        }
        result
    }

    pub(super) fn mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_grid_layout(cx);
        window.focus(&self.focus, cx);
        let point = ev.position - self.layout.bounds.origin;
        let show_scrollbars = self.show_scrollbars(cx);
        let effect = self.with_grid(cx, |grid, geometry| {
            grid.pointer_down(
                point,
                ev.modifiers.shift,
                ev.click_count,
                geometry,
                show_scrollbars,
            )
        });
        match effect {
            Some(GridEffect::CycleSort { column, multiple }) => {
                self.cycle_sort(column, multiple, cx)
            }
            Some(GridEffect::EditFilter(column)) => {
                self.filter_column(self.data.columns[column].id, window, cx)
            }
            Some(GridEffect::Autofit(column)) => self.autofit(column, window),
            Some(GridEffect::ShowCell(cell)) => self.show_cell(cell, window, cx),
            None => {}
        }
        cx.notify();
    }

    pub(super) fn mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_grid_layout(cx);
        let point = ev.position - self.layout.bounds.origin;
        if ev.pressed_button != Some(MouseButton::Left) {
            let show_scrollbars = self.show_scrollbars(cx);
            if self
                .grid
                .pointer_hover(point, &self.layout, show_scrollbars)
            {
                cx.notify();
            }
            return;
        }
        let update = self.with_grid(cx, |grid, geometry| grid.pointer_drag(point, geometry));
        self.apply_drag(update, window, cx);
    }

    fn apply_drag(
        &mut self,
        update: Option<DragUpdate>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match update {
            None => return,
            Some(DragUpdate::Moved) => {}
            Some(DragUpdate::Resize { column, width }) => {
                self.data
                    .columns
                    .set_width(column, ColumnWidth::Manual(width));
            }
            Some(DragUpdate::EdgeScrolling) if !self.edge_frame_pending => {
                // Keep extending the selection on every frame while the pointer rests there.
                self.edge_frame_pending = true;
                let entity = cx.weak_entity();
                window.on_next_frame(move |window, cx| {
                    let _ = entity.update(cx, |table, cx| {
                        table.edge_frame_pending = false;
                        let update =
                            table.with_grid(cx, |grid, geometry| grid.edge_scroll(geometry));
                        table.apply_drag(update, window, cx);
                    });
                });
            }
            Some(DragUpdate::EdgeScrolling) => {}
        }
        cx.notify();
    }

    pub(super) fn mouse_left(&mut self, hovering: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if !hovering && self.grid.pointer_left() {
            cx.notify();
        }
    }

    pub(super) fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.grid.pointer_up();
        cx.notify();
    }

    pub(super) fn scroll_wheel(
        &mut self,
        ev: &ScrollEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_grid_layout(cx);
        // Trackpads send exact pixels, including the platform's momentum events
        // after the fingers lift (phase `Moved`); wheels send lines.
        let delta = ev.delta.pixel_delta(px(self.prefs.row_height));
        let (dx, dy) = (f64::from(delta.x.as_f32()), f64::from(delta.y.as_f32()));
        self.flash_scrollbars(cx);
        self.grid
            .wheel(dx, dy, ev.modifiers.shift, &self.data.columns, &self.layout);
        cx.notify();
        cx.stop_propagation();
    }

    fn flash_scrollbars(&mut self, cx: &mut Context<Self>) {
        if cx.should_auto_hide_scrollbars() {
            let timer = cx
                .background_executor()
                .timer(std::time::Duration::from_millis(650));
            self.scrollbar_timer = Some(cx.spawn(async move |table, cx| {
                timer.await;
                let _ = table.update(cx, |table, cx| {
                    table.scrollbar_timer = None;
                    cx.notify();
                });
            }));
        }
    }

    pub(super) fn navigate(&mut self, action: &Navigate, _: &mut Window, cx: &mut Context<Self>) {
        self.commit_grid_layout(cx);
        let rows = self.data.row_count();
        let scrolled = self.with_grid(cx, |grid, geometry| {
            grid.navigate(
                action.motion,
                action.extend,
                rows,
                geometry.columns,
                geometry.layout,
            )
        });
        if scrolled {
            self.flash_scrollbars(cx);
        }
        cx.notify();
    }
}
