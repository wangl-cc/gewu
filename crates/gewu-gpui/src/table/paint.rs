use super::{layout::FUNNEL_WIDTH, *};

pub(super) fn paint(frame: Frame, window: &mut Window, cx: &mut App) {
    let l = &frame.layout;
    let origin = l.bounds.origin;
    let (w, h) = (l.bounds.size.width.as_f32(), l.bounds.size.height.as_f32());
    let colors = crate::theme::GewuPalette::get(cx);
    let cues = crate::theme::VisualCues::get(cx);
    let active = window.is_window_active();
    let selection = if active {
        colors.selection_active
    } else {
        colors.selection_inactive
    };
    let header_selection = if active {
        colors.button_secondary
    } else {
        colors
            .button_secondary
            .alpha(colors.button_secondary.a * 0.55)
    };
    let focus = if active {
        colors.focus_ring
    } else {
        colors.muted_foreground
    };
    let hairline = 1. / window.scale_factor();
    let (bg, stripe, grid, header_bg) = (colors.canvas, colors.stripe, colors.grid, colors.surface);
    let (text_color, muted) = (colors.foreground, colors.muted_foreground);
    let font_size = frame.style.font_size;
    let row_height = frame.style.row_height;
    let mut font = window.text_style().font();
    font.features = crate::theme::tabular_digits();
    let run = |len: usize, color: Hsla| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let rect = |x: f32, y: f32, w: f32, h: f32| {
        Bounds::new(origin + point(px(x), px(y)), size_(w.max(0.), h.max(0.)))
    };
    let row_area = |x, width| rect(x, HEADER_HEIGHT, width, l.body_bottom() - HEADER_HEIGHT);
    let text_y = |y: f32| y + (row_height - font_size * 1.3) / 2.;
    let line_height = px(font_size * 1.3);

    window.paint_quad(fill(rect(0., 0., w, h), bg));
    // Cells, one content mask per column.
    let body = row_area(l.gutter, l.body_right() - l.gutter);
    window.with_content_mask(Some(ContentMask { bounds: body }), |window| {
        for row in frame.rows.iter().filter(|row| row.display.0 % 2 == 1) {
            window.paint_quad(fill(
                rect(l.gutter, row.y, w - l.gutter, row_height),
                stripe,
            ));
        }
        if let Some(y) = frame.hovered_row_y {
            window.paint_quad(fill(
                rect(l.gutter, y, w - l.gutter, row_height),
                colors.stripe,
            ));
        }
        if let Some(bounds) = frame.selection {
            window.paint_quad(fill(bounds + origin, selection));
        }
        for PaintColumn {
            x,
            width,
            right_aligned,
            cells,
        } in &frame.columns
        {
            window.paint_quad(fill(row_area(x + width - hairline, hairline), grid));
            let mask = row_area(x + PAD, width - 2. * PAD);
            window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
                for PaintCell {
                    y,
                    text,
                    missing,
                    current_match,
                    spans,
                } in cells
                {
                    let mut text_run = run(text.len(), if *missing { muted } else { text_color });
                    if *missing {
                        text_run.font.style = FontStyle::Italic;
                    }
                    let (mut line, fitted_bytes) =
                        fit_line(text, px(width - 2. * PAD), px(font_size), &text_run, window);
                    if !spans.is_empty() {
                        line = super::find_style::matched_line(
                            line,
                            fitted_bytes,
                            &frame.text_matches[spans.clone()],
                            &text_run,
                            window,
                            if *current_match {
                                colors.find_span_current
                            } else {
                                colors.find_span
                            },
                        );
                    }
                    let tx = if *right_aligned {
                        x + width - PAD - line.width.as_f32()
                    } else {
                        x + PAD
                    };
                    let position = origin + point(px(tx), px(text_y(*y)));
                    if !spans.is_empty() {
                        let _ = line.paint_background(
                            position,
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                    line.paint(position, line_height, TextAlign::Left, None, window, cx)
                        .ok();
                }
            });
        }
        if let Some(hit) = frame.current_match
            && frame.head != Some(hit)
        {
            let bounds = (hit + origin).inset(cues.cell_outline);
            window.paint_quad(
                outline(bounds, colors.find_current_border, BorderStyle::Solid)
                    .border_widths(cues.find_outline),
            );
        }
        if let Some(head) = frame.head {
            window.paint_quad(
                outline(head + origin, focus, BorderStyle::Solid).border_widths(cues.cell_outline),
            );
        }
    });

    // Row number gutter with source rows.
    let gutter = row_area(0., l.gutter);
    window.paint_quad(fill(gutter, header_bg));
    window.paint_quad(fill(row_area(l.gutter - hairline, hairline), grid));
    window.with_content_mask(Some(ContentMask { bounds: gutter }), |window| {
        for FrameRow {
            y, label, selected, ..
        } in &frame.rows
        {
            if *selected {
                window.paint_quad(fill(
                    rect(0., *y, l.gutter - hairline, row_height),
                    header_selection,
                ));
            }
            let row_ink = if *selected && active {
                text_color
            } else {
                muted
            };
            let line = window.text_system().shape_line(
                label.clone(),
                px(ROW_NUMBER_SIZE),
                &[run(label.len(), row_ink)],
                None,
            );
            let tx = l.gutter - PAD - line.width.as_f32();
            line.paint(
                origin + point(px(tx), px(*y + (row_height - 15.) / 2.)),
                px(15.),
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
        }
    });

    // Header.
    window.paint_quad(fill(rect(0., 0., w, HEADER_HEIGHT), header_bg));
    window.paint_quad(fill(rect(0., HEADER_HEIGHT - hairline, w, hairline), grid));
    let header_mask = rect(l.gutter, 0., l.body_right() - l.gutter, HEADER_HEIGHT);
    window.with_content_mask(
        Some(ContentMask {
            bounds: header_mask,
        }),
        |window| {
            for ((column, x, width), header) in l.columns.iter().zip(&frame.header) {
                let right = x + width;
                if header.selected {
                    window.paint_quad(fill(
                        rect(*x, 0., *width, HEADER_HEIGHT - hairline),
                        header_selection,
                    ));
                } else if frame.header_hover == Some(HeaderHover::Column(*column)) {
                    window.paint_quad(fill(
                        rect(*x, 0., *width, HEADER_HEIGHT - hairline),
                        colors.button_secondary_hover,
                    ));
                }
                let (thickness, color) =
                    if frame.header_hover == Some(HeaderHover::Divider(*column)) {
                        (1.5, colors.muted_foreground)
                    } else {
                        (hairline, grid)
                    };
                window.paint_quad(fill(
                    rect(right - thickness, 0., thickness, HEADER_HEIGHT),
                    color,
                ));
                let available = px((width - header::HEADER_INSET).max(0.));
                let indicator = header.sort.as_ref().map(|label| {
                    window.text_system().shape_line(
                        label.clone(),
                        px(header::SORT_SIZE),
                        &[run(label.len(), text_color)],
                        None,
                    )
                });
                let name_width = indicator.as_ref().map_or(available, |line| {
                    (available - line.width - px(header::SORT_GAP)).max(px(0.))
                });
                let mut name_run = run(header.name.len(), text_color);
                name_run.font.weight = FontWeight::MEDIUM;
                let (label, _) = fit_line(
                    &header.name,
                    name_width,
                    px(header::NAME_SIZE),
                    &name_run,
                    window,
                );
                let (kind, _) = fit_line(
                    &header.kind,
                    available,
                    px(header::TYPE_SIZE),
                    &run(header.kind.len(), muted),
                    window,
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: rect(x + PAD, 0., width - 2. * PAD, HEADER_HEIGHT),
                    }),
                    |window| {
                        let _ = label.paint(
                            origin + point(px(x + PAD), px(1.)),
                            px(17.),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                        let _ = kind.paint(
                            origin + point(px(x + PAD), px(17.)),
                            px(13.),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                        if let Some(line) = indicator {
                            let at = px(x + PAD) + available - line.width;
                            let _ = line.paint(
                                origin + point(at, px(1.)),
                                px(17.),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        if let Some(active) = header.funnel {
                            let color = if active { colors.accent } else { muted };
                            let left = Layout::funnel_left(right);
                            for (dy, line_width) in
                                [(0., FUNNEL_WIDTH), (2., 7.), (4., 4.), (6., 2.)]
                            {
                                window.paint_quad(fill(
                                    rect(
                                        left + (FUNNEL_WIDTH - line_width) / 2.,
                                        11. + dy,
                                        line_width,
                                        2.,
                                    ),
                                    color,
                                ));
                            }
                        }
                    },
                );
            }
        },
    );

    // Keep the hit target independent of the visible thumb's thickness.
    for axis in [Axis::Horizontal, Axis::Vertical] {
        let Some(bar) = l.scrollbar(axis) else {
            continue;
        };
        let active = frame.active_thumb == Some(axis);
        if !frame.show_scrollbars && !active {
            continue;
        }
        let cross = axis.invert();
        let target = bar.thumb.size.along(cross);
        let thickness = px(if active { 11. } else { 7. }).min(target);
        let bounds = Bounds::new(
            origin
                + bar
                    .thumb
                    .origin
                    .apply_along(cross, |start| start + (target - thickness) / 2.),
            bar.thumb.size.apply_along(cross, |_| thickness),
        );
        let alpha = if active { 0.8 } else { 0.55 };
        window.paint_quad(fill(bounds, muted.alpha(alpha)).corner_radii(thickness / 2.));
    }
}

/// Return the fitted line and the number of original UTF-8 bytes it still displays.
fn fit_line(
    text: &SharedString,
    width: Pixels,
    font_size: Pixels,
    run: &TextRun,
    window: &Window,
) -> (ShapedLine, usize) {
    let runs = std::slice::from_ref(run);
    let system = window.text_system();
    let line = system.shape_line(text.clone(), font_size, runs, None);
    if line.width <= width {
        return (line, text.len());
    }
    let Some(end) = system
        .line_wrapper(run.font.clone(), font_size)
        .should_truncate_line(text, width.max(px(0.)), "…", TruncateFrom::End)
    else {
        return (line, text.len());
    };
    let label: SharedString = format!("{}…", &text[..end]).into();
    let run = TextRun {
        len: label.len(),
        ..run.clone()
    };
    (system.shape_line(label, font_size, &[run], None), end)
}
