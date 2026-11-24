use std::{
    fs,
    io::{self, BufRead, Write as _},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use gpui_kit::{
    HeadlessAppContext,
    test::{TestEventExt, TestWindowExt},
};
use serde_json::{Value, json};

use super::*;

pub(crate) fn run() {
    let mut app = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(assets::AppAssets),
        gpui_kit::platform::current_headless_renderer,
    );
    let (_send, recv) = async_channel::unbounded();
    let path = PathBuf::from(std::env::args_os().nth(1).expect("dataset argument"));
    let mut handle = app.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        cx.set_global(crate::preferences::Preferences::default());
        install_file_actions(platform::RecentDocuments::Session(Vec::new()), recv, cx);
        open_paths(vec![path], cx);
        *cx.windows()
            .first()
            .expect("open document. This host requires macOS graphics access")
    });
    let document = handle;
    let mut settings = None;
    let mut size = size(px(1280.), px(800.));
    let mut scale = 1.;
    let mut appearance = WindowAppearance::Light;
    let mut active = true;
    for command in
        std::iter::once("init".to_string()).chain(io::stdin().lock().lines().map_while(Result::ok))
    {
        let (action, args) = command.split_once(' ').unwrap_or((&command, ""));
        match action {
            "init" | "output" | "resize" => {
                if action == "output" {
                    let v: Vec<_> = args.split_whitespace().collect();
                    appearance = if v[0] == "dark" {
                        WindowAppearance::Dark
                    } else {
                        WindowAppearance::Light
                    };
                    active = v[1] == "active";
                    scale = v[2].parse().unwrap();
                }
                if action == "resize" {
                    let v: Vec<_> = args.split_whitespace().collect();
                    size = gpui_kit::size(px(v[0].parse().unwrap()), px(v[1].parse().unwrap()));
                }
                app.review_window(handle, scale, appearance, active, size);
            }
            "key" | "input" | "left" | "right" | "drag" | "action" => {
                app.update_window(handle, |_, w, cx| dispatch(action, args, w, cx))
                    .unwrap();
            }
            "settings" => {
                let before = app.update(|cx| cx.windows());
                app.update(crate::preferences::open);
                handle = *settings.get_or_insert_with(|| {
                    app.update(|cx| {
                        cx.windows()
                            .into_iter()
                            .find(|h| !before.contains(h))
                            .unwrap()
                    })
                });
                size = app
                    .update_window(handle, |_, w, _| w.viewport_size())
                    .unwrap();
                app.review_window(handle, scale, appearance, active, size);
            }
            "document" => {
                handle = document;
                size = gpui_kit::size(px(1280.), px(800.));
                app.review_window(handle, scale, appearance, active, size);
            }
            "snap" => {}
            _ => panic!("unknown command {action}"),
        }
        app.run_until_parked();
        app.advance_clock(Duration::from_millis(280));
        for _ in 0..3 {
            app.run_until_parked();
            app.update_window(handle, |_, w, cx| w.render_frame(cx))
                .unwrap();
        }
        let state = app
            .update_window(handle, |_, window, cx| evidence(window, cx))
            .expect("open capture window");
        if action == "snap" {
            let path = PathBuf::from(args);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            use image::{
                ImageEncoder,
                codecs::png::{CompressionType, FilterType, PngEncoder},
            };
            let image = app.capture_screenshot(handle).unwrap();
            let writer = std::io::BufWriter::new(fs::File::create(path).unwrap());
            PngEncoder::new_with_quality(writer, CompressionType::Best, FilterType::Adaptive)
                .write_image(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
        }
        println!("{state}");
        io::stdout().flush().unwrap();
    }
}

/// Capture state from the same completed frame that produced the PNG.
fn evidence(window: &Window, cx: &App) -> Value {
    use crate::theme::GewuPalette;

    let current = gpui_kit::base::Theme::global(cx).appearance;
    let installed = GewuPalette::get(cx);
    let palette = if installed == GewuPalette::default_for(current) {
        "standard"
    } else if installed == GewuPalette::high_contrast_for(current) {
        "high-contrast"
    } else {
        panic!("unexpected palette");
    };
    let update = window.review_accessibility().expect("GPUI_A11Y_ACTIVE=1");
    let consumer = accesskit_consumer::Tree::new(update.clone(), window.is_window_active());
    let nodes: Vec<_> = update
        .nodes
        .iter()
        .filter_map(|(id, raw)| {
            let bounds = raw.bounds()?;
            let node = consumer
                .state()
                .node_by_tree_local_id(*id, update.tree_id)?;
            Some(json!({
                "a": {
                    "label": node.label(),
                    "role": format!("{:?}", node.role()),
                    "description": node.description(),
                    "toggled": node.toggled().map(|v| format!("{v:?}")),
                },
                "rect": [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
            }))
        })
        .collect();
    let viewport = window.viewport_size();
    json!({
        "nodes": nodes,
        "palette": palette,
        "scale": window.scale_factor(),
        "active": window.is_window_active(),
        "appearance": format!("{:?}", window.appearance()),
        "width": viewport.width.as_f32(),
        "height": viewport.height.as_f32(),
    })
}

fn dispatch(action: &str, args: &str, w: &mut Window, cx: &mut App) {
    match action {
        "key" => w.press(args, cx),
        "input" => w.input(args, cx),
        "left" | "right" => {
            let coords: Vec<f32> = args
                .split_whitespace()
                .map(|x| x.parse().unwrap())
                .collect();
            let pos = point(px(coords[0]), px(coords[1]));
            let button = if action == "left" {
                MouseButton::Left
            } else {
                MouseButton::Right
            };
            w.pointer_move(pos, None, cx);
            w.pointer_down(pos, button, cx);
            w.pointer_up(pos, button, cx);
        }
        "drag" => {
            let c: Vec<f32> = args
                .split_whitespace()
                .map(|x| x.parse().unwrap())
                .collect();
            w.drag(point(px(c[0]), px(c[1])), point(px(c[2]), px(c[3])), cx);
        }
        _ => {
            let a: Box<dyn Action> = match args {
                "columns" => Box::new(crate::actions::ManageColumns),
                "sort" => Box::new(crate::actions::EditSort),
                "filter" => Box::new(crate::actions::EditFilter),
                "chart" => Box::new(crate::actions::ShowChartInspector),
                "analyze" => Box::new(crate::actions::AnalyzeSelectedColumn),
                _ => panic!("unknown action {args}"),
            };
            w.dispatch_action(a, cx);
        }
    }
}
