use super::*;

impl TableView {
    pub(super) fn divider_at(&self, x: f32) -> Option<usize> {
        self.layout
            .columns
            .iter()
            .find(|&&(_, cx, w)| (x - (cx + w)).abs() <= RESIZE_SLOP)
            .map(|c| c.0)
    }

    pub(super) fn mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_grid_layout(cx);
        window.focus(&self.focus, cx);
        let local = ev.position - self.layout.bounds.origin;
        let (x, y) = (local.x.as_f32(), local.y.as_f32());
        let l = &self.layout;
        if let Some((axis, bar)) = self.scrollbar_at(local, self.show_scrollbars(cx)) {
            let at = local.along(axis);
            let grab = if bar.thumb.contains(&local) {
                at - bar.thumb.origin.along(axis)
            } else {
                let half = bar.thumb.size.along(axis) / 2.;
                self.move_scrollbar(axis, &bar, at - half);
                half
            };
            self.drag = Drag::Scrollbar { axis, grab };
        } else if (0.0..HEADER_HEIGHT).contains(&y) && (l.gutter..l.body_right()).contains(&x) {
            if let Some(column) = self.divider_at(x) {
                if ev.click_count == 2 {
                    self.autofit(column, window);
                } else {
                    self.drag = Drag::Resize {
                        column,
                        start_x: x,
                        start_width: self.data.columns.width(column),
                    };
                }
            } else if let Some(column) = l.column_at(x) {
                if self
                    .column_left(column)
                    .is_some_and(|left| x >= left + self.data.columns.width(column) - PAD - 14.)
                {
                    self.filter_column(self.data.columns[column].id, window, cx);
                    return;
                }
                self.cycle_sort(column, ev.modifiers.shift, cx);
            }
        } else if let Some(cell) = l.cell_at(x, y) {
            let selection = match (self.selection, ev.modifiers.shift) {
                (Some(s), true) => Selection {
                    anchor: s.anchor,
                    head: cell,
                },
                _ => Selection::single(cell),
            };
            self.set_selection(Some(selection), cx);
            if ev.click_count == 2 {
                self.show_cell(cell, window, cx);
            } else {
                self.drag = Drag::Select { edge_pointer: None };
            }
        }
        cx.notify();
    }

    pub(super) fn show_scrollbars(&self, cx: &App) -> bool {
        !cx.should_auto_hide_scrollbars() || self.scrollbar_timer.is_some()
    }

    /// Hidden overlay tracks pass pointer input through to the cell underneath.
    pub(super) fn scrollbar_at(
        &self,
        point: Point<Pixels>,
        show_all: bool,
    ) -> Option<(Axis, layout::Scrollbar)> {
        [Axis::Vertical, Axis::Horizontal]
            .into_iter()
            .find_map(|axis| {
                if !show_all && self.thumb_hover != Some(axis) {
                    return None;
                }
                let bar = self.layout.scrollbar(axis)?;
                bar.track.contains(&point).then_some((axis, bar))
            })
    }

    fn move_scrollbar(&mut self, axis: Axis, bar: &layout::Scrollbar, start: Pixels) {
        let travel = bar.track.size.along(axis) - bar.thumb.size.along(axis);
        if travel > px(0.) {
            let fraction = ((start - bar.track.origin.along(axis)) / travel).clamp(0., 1.);
            match axis {
                Axis::Vertical => self.scroll.set_fraction(f64::from(fraction)),
                Axis::Horizontal => {
                    self.horizontal =
                        HorizontalScroll::Offset(fraction * self.layout.horizontal_overflow())
                }
            }
        }
    }

    pub(super) fn mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_grid_layout(cx);
        let local = ev.position - self.layout.bounds.origin;
        if ev.pressed_button != Some(MouseButton::Left) {
            let (x, y) = (local.x.as_f32(), local.y.as_f32());
            let hover = if (0.0..HEADER_HEIGHT).contains(&y)
                && (self.layout.gutter..self.layout.body_right()).contains(&x)
            {
                self.divider_at(x)
                    .map(HeaderHover::Divider)
                    .or_else(|| self.layout.column_at(x).map(HeaderHover::Column))
            } else {
                None
            };
            let thumb = self
                .scrollbar_at(local, self.show_scrollbars(cx))
                .map(|(axis, _)| axis);
            let body = if thumb.is_none() {
                self.layout.cell_at(x, y)
            } else {
                None
            };
            let previous = self
                .body_pointer
                .and_then(|point| self.layout.cell_at(point.x.as_f32(), point.y.as_f32()));
            self.body_pointer = body.map(|_| local);
            if self.header_hover != hover
                || self.thumb_hover != thumb
                || previous.map(|c| c.row) != body.map(|c| c.row)
            {
                self.header_hover = hover;
                self.thumb_hover = thumb;
                cx.notify();
            }
            return;
        }
        let x = local.x.as_f32();
        match self.drag {
            Drag::None => return,
            Drag::Scrollbar { axis, grab } => {
                if let Some(bar) = self.layout.scrollbar(axis) {
                    self.move_scrollbar(axis, &bar, local.along(axis) - grab);
                }
            }
            Drag::Resize {
                column,
                start_x,
                start_width,
            } => {
                self.data.columns.set_width(
                    column,
                    ColumnWidth::Manual((start_width + x - start_x).max(MIN_COLUMN)),
                );
            }
            Drag::Select { .. } => self.extend_drag(local, window, cx),
        }
        cx.notify();
    }

    fn extend_drag(&mut self, local: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = (local.x.as_f32(), local.y.as_f32());
        let edge_delta = |coord: f32, min: f32, max: f32| {
            let margin = 8_f32.min(((max - min) / 2.).max(0.));
            if coord < min + margin {
                (coord - min - margin).max(-64.)
            } else if coord > max - margin {
                (coord - max + margin).min(64.)
            } else {
                0.
            }
        };
        let dx = edge_delta(x, self.layout.gutter, self.layout.body_right());
        let dy = edge_delta(y, HEADER_HEIGHT, self.layout.body_bottom());
        let before = (self.scroll.anchor(), self.scroll_x());
        self.scroll.scroll_by(f64::from(dy));
        self.horizontal = HorizontalScroll::Offset((before.1 + dx).max(0.));
        let layout = self.compute_layout(self.layout.bounds);
        self.horizontal = HorizontalScroll::Offset(layout.scroll_x);
        self.scroll = layout.vertical.clone();
        if let (Some(selection), Some(head)) = (self.selection, layout.drag_cell_at(x, y)) {
            self.set_selection(Some(Selection { head, ..selection }), cx);
        }
        let moved = before != (self.scroll.anchor(), self.scroll_x());
        self.drag = Drag::Select {
            edge_pointer: moved.then_some(local),
        };
        if moved && !self.edge_frame_pending {
            self.edge_frame_pending = true;
            let entity = cx.weak_entity();
            window.on_next_frame(move |w, cx| {
                let _ = entity.update(cx, |t, cx| {
                    t.edge_frame_pending = false;
                    if let Drag::Select {
                        edge_pointer: Some(local),
                    } = t.drag
                    {
                        t.extend_drag(local, w, cx);
                        cx.notify();
                    }
                });
            });
        }
    }

    pub(super) fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.drag = Drag::None;
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
        let (mut dx, mut dy) = (delta.x.as_f32() as f64, delta.y.as_f32() as f64);
        if ev.modifiers.shift && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        self.flash_scrollbars(cx);
        self.scroll.scroll_by(-dy);
        if dx != 0. {
            self.horizontal = HorizontalScroll::Offset((self.scroll_x() - dx as f32).max(0.));
        }
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
        let columns = self.data.columns.len();
        if rows == 0 || columns == 0 {
            return;
        }
        let current = self
            .selection
            .unwrap_or(Selection::single(Cell { row: 0, column: 0 }));
        let page = self.scroll.page_rows() as i64;
        let far = i64::MAX / 2;
        let (dr, dc) = match action.motion {
            Motion::Up => (-1, 0),
            Motion::Down => (1, 0),
            Motion::Left => (0, -1),
            Motion::Right => (0, 1),
            Motion::FirstRow => (-far, 0),
            Motion::LastRow => (far, 0),
            Motion::FirstColumn => (0, -far),
            Motion::LastColumn => (0, far),
            Motion::PageUp => (-page, 0),
            Motion::PageDown => (page, 0),
        };
        let next = current.moved(dr, dc, action.extend, rows, columns);
        self.set_selection(Some(next), cx);
        let before = (self.scroll.anchor(), self.scroll_x());
        self.reveal(next.head);
        if before != (self.scroll.anchor(), self.scroll_x()) {
            self.flash_scrollbars(cx);
        }
        cx.notify();
    }
}
