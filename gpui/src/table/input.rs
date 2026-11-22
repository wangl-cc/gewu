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
        window.focus(&self.focus, cx);
        let local = ev.position - self.layout.bounds.origin;
        let (x, y) = (local.x.as_f32(), local.y.as_f32());
        let l = &self.layout;
        if l.has_horizontal_scroll() && y >= l.body_bottom() && x >= l.gutter && x < l.body_right()
        {
            let thumb = l.horizontal_thumb();
            let grab = if thumb.contains(&local) {
                x - thumb.origin.x.as_f32()
            } else {
                thumb.size.width.as_f32() / 2.
            };
            self.drag_horizontal_thumb(x - grab);
            self.drag = Drag::HorizontalThumb { grab };
        } else if y >= HEADER_HEIGHT
            && y <= l.body_bottom()
            && x >= l.body_right()
            && self.scroll.max_position_rows() > 0.
        {
            let thumb = l.vertical_thumb();
            if thumb.contains(&local) {
                self.drag = Drag::Thumb {
                    grab: y - thumb.origin.y.as_f32(),
                };
            } else {
                // Track click: jump there, centered on the thumb.
                self.drag_thumb(y - thumb.size.height.as_f32() / 2.);
                self.drag = Drag::Thumb {
                    grab: thumb.size.height.as_f32() / 2.,
                };
            }
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
            self.selection = Some(match (self.selection, ev.modifiers.shift) {
                (Some(s), true) => Selection {
                    anchor: s.anchor,
                    head: cell,
                },
                _ => Selection::single(cell),
            });
            if ev.click_count == 2 {
                self.show_cell(cell, cx);
            } else {
                self.drag = Drag::Select;
            }
        }
        cx.notify();
    }

    pub(super) fn drag_horizontal_thumb(&mut self, left: f32) {
        let l = &self.layout;
        let track = (l.body_right() - l.gutter).max(0.);
        let travel = track - l.horizontal_thumb().size.width.as_f32();
        if travel > 0. {
            self.scroll_x =
                ((left - l.gutter) / travel).clamp(0., 1.) * (l.content_width - track).max(0.);
        }
    }

    pub(super) fn drag_thumb(&mut self, top: f32) {
        let l = &self.layout;
        let track = l.body_bottom() - HEADER_HEIGHT;
        let size = l.vertical_thumb().size.height.as_f32();
        let fraction = ((top - HEADER_HEIGHT) / (track - size).max(1.)) as f64;
        self.scroll.set_fraction(fraction);
    }

    pub(super) fn mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let local = ev.position - self.layout.bounds.origin;
        if ev.pressed_button != Some(MouseButton::Left) {
            let hover = (0.0..HEADER_HEIGHT)
                .contains(&local.y.as_f32())
                .then(|| self.layout.column_at(local.x.as_f32()))
                .flatten();
            let (x, y) = (local.x.as_f32(), local.y.as_f32());
            let body = self.layout.cell_at(x, y);
            let previous = self
                .body_pointer
                .and_then(|point| self.layout.cell_at(point.x.as_f32(), point.y.as_f32()));
            self.body_pointer = body.map(|_| local);
            let thumb = if self.layout.has_horizontal_scroll()
                && (self.layout.gutter..self.layout.body_right()).contains(&x)
                && y >= self.layout.body_bottom()
            {
                Some(Thumb::Horizontal)
            } else if self.layout.vertical.max_position_rows() > 0.
                && x >= self.layout.body_right()
                && (HEADER_HEIGHT..self.layout.body_bottom()).contains(&y)
            {
                Some(Thumb::Vertical)
            } else {
                None
            };
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
        let (x, y) = (local.x.as_f32(), local.y.as_f32());
        match self.drag {
            Drag::None => return,
            Drag::HorizontalThumb { grab } => self.drag_horizontal_thumb(x - grab),
            Drag::Thumb { grab } => self.drag_thumb(y - grab),
            Drag::Resize {
                column,
                start_x,
                start_width,
            } => {
                self.data.columns.entry_mut(column).width =
                    ColumnWidth::Manual((start_width + x - start_x).max(MIN_COLUMN));
            }
            Drag::Select => {
                self.edge_pointer = Some(local);
                self.extend_drag(window, cx);
            }
        }
        cx.notify();
    }

    fn extend_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(local) = self.edge_pointer else {
            return;
        };
        if !matches!(self.drag, Drag::Select) {
            self.edge_pointer = None;
            return;
        }
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
        let before = (self.scroll.anchor(), self.scroll_x);
        self.scroll.scroll_by(f64::from(dy));
        self.scroll_x = (self.scroll_x + dx).max(0.);
        let layout = self.compute_layout(self.layout.bounds);
        self.scroll_x = layout.scroll_x;
        self.scroll = layout.vertical.clone();
        if let (Some(selection), Some(head)) = (&mut self.selection, layout.drag_cell_at(x, y)) {
            selection.head = head;
        }
        if before == (self.scroll.anchor(), self.scroll_x) {
            self.edge_pointer = None;
            return;
        }
        if !self.edge_frame_pending {
            self.edge_frame_pending = true;
            let entity = cx.weak_entity();
            window.on_next_frame(move |w, cx| {
                let _ = entity.update(cx, |t, cx| {
                    t.edge_frame_pending = false;
                    if t.edge_pointer.is_some() {
                        t.extend_drag(w, cx);
                        cx.notify();
                    }
                });
            });
        }
    }

    pub(super) fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.drag = Drag::None;
        self.edge_pointer = None;
        cx.notify();
    }

    pub(super) fn scroll_wheel(
        &mut self,
        ev: &ScrollEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Trackpads send exact pixels, including the platform's momentum events
        // after the fingers lift (phase `Moved`); wheels send lines.
        let delta = ev.delta.pixel_delta(px(self.prefs.row_height));
        let (mut dx, mut dy) = (delta.x.as_f32() as f64, delta.y.as_f32() as f64);
        if ev.modifiers.shift && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        self.flash_scrollbars(cx);
        self.scroll.scroll_by(-dy);
        self.scroll_x = (self.scroll_x - dx as f32).max(0.);
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
        self.selection = Some(next);
        let before = (self.scroll.anchor(), self.scroll_x);
        self.reveal(next.head);
        if before != (self.scroll.anchor(), self.scroll_x) {
            self.flash_scrollbars(cx);
        }
        cx.notify();
    }
}
