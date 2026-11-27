use std::time::Duration;

use gewu_gpui::examples::Document;
use gpui_kit::{
    AppContext, Bounds, OperatingSystem, PlatformInfo, PlatformTarget, Point, TestAppContext,
    VisualTestContext, WindowBounds, WindowOptions, px, size, test::TestWindowExt,
};

use super::{Driver, frame};

#[test]
fn retina_grid_fits_inside_a_whole_point_window() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.csv");
    let mut csv = String::from("a,b,c,d,e,f,g,h,i,j\n");
    for _ in 0..150 {
        csv.push_str("1,2,3,4,5,6,7,8,9,10\n");
    }
    std::fs::write(&path, csv).unwrap();
    let mut app = TestAppContext::single();
    app.set_platform_info(PlatformInfo {
        os: OperatingSystem::Mac,
        target: PlatformTarget::Native,
    });
    let (handle, driver) = app.update(|cx| {
        gewu_gpui::examples::init(cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Point::default(),
                size(px(1280.), px(866.)),
            ))),
            ..Default::default()
        };
        gpui_kit::open_window(opts, cx, |w, cx| {
            let document = Document::open(path.clone(), w, cx);
            cx.new(|_| Driver::new(document, 150, Duration::from_secs(1)))
        })
        .unwrap()
    });
    let mut ui = VisualTestContext::from_window(handle, &app);
    ui.simulate_scale_factor_change(2.);
    ui.run_until_parked();
    let (native, target) = ui.update(|w, cx| {
        w.render_frame(cx);
        let native = w.viewport_size();
        assert_eq!(native.height.as_f32().fract(), 0.);
        let grid = frame::Grid::read(w).unwrap().size;
        let target = size(grid.width, grid.height - px(0.5));
        driver.update(cx, |d, cx| d.fit_grid(grid, target, w, cx));
        (native, target)
    });
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert_eq!(w.scale_factor(), 2.);
        assert_eq!(w.viewport_size(), native);
        assert_eq!(frame::Grid::read(w).unwrap().size, target);
    });
    // Now the fixture also needs a horizontal track. The benchmark's requested
    // grid height still includes the space occupied by that track.
    ui.simulate_resize(size(px(600.), native.height));
    ui.run_until_parked();
    ui.update(|w, cx| {
        w.render_frame(cx);
        assert_eq!(
            frame::Grid::read(w).unwrap().size,
            size(px(600.), target.height)
        );
    });
    app.quit();
}
