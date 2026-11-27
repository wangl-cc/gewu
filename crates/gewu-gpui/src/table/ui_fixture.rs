use std::path::Path;

use gpui_kit::{
    AppContext, Bounds, Entity, Modifiers, OperatingSystem, Pixels, PlatformInfo, PlatformTarget,
    Point, TestAppContext, VisualTestContext, WindowBounds, WindowOptions, point, px, size,
    test::TestWindowExt,
};

use super::{HEADER_HEIGHT, ROW_HEIGHT, TableView};
use crate::{
    actions,
    table::{data::TableData, selection::Selection},
};

pub(super) struct TableWindow {
    pub app: TestAppContext,
    pub ui: VisualTestContext,
    pub table: Entity<TableView>,
}

impl TableWindow {
    pub fn open(path: &Path) -> Self {
        let mut app = TestAppContext::single();
        app.set_platform_info(PlatformInfo {
            os: OperatingSystem::Mac,
            target: PlatformTarget::Native,
        });
        let (window, table) = app.update(|cx| {
            gpui_kit::init(cx);
            actions::init(cx);
            cx.set_global(crate::preferences::Preferences {
                row_height: ROW_HEIGHT,
                show_inspector: false,
                copy_includes_headers: false,
                ..Default::default()
            });
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Point::default(),
                    size(px(680.), px(420.)),
                ))),
                ..Default::default()
            };
            let data = TableData::open(path.to_str().unwrap()).unwrap();
            gpui_kit::open_window(options, cx, |window, cx| {
                let table = cx.new(|cx| TableView::new(data, window, cx));
                let focus = table.read(cx).focus.clone();
                window.focus(&focus, cx);
                table
            })
            .unwrap()
        });
        let ui = VisualTestContext::from_window(window, &app);
        let mut window = Self { app, ui, table };
        window.settle();
        window
    }

    pub fn read<R>(&self, f: impl FnOnce(&TableView) -> R) -> R {
        self.app.read(|cx| f(self.table.read(cx)))
    }

    pub fn settle(&mut self) {
        // A newly installed query may request Find highlights for its visible rows.
        self.ui.run_until_parked();
        self.ui.update(|window, cx| window.render_frame(cx));
        self.ui.run_until_parked();
        self.ui.update(|window, cx| window.render_frame(cx));
        self.ui.run_until_parked();
    }

    pub fn type_in_find(&mut self, text: &str) {
        self.ui.update(|window, cx| window.input(text, cx));
        self.ui.run_until_parked();
        self.app
            .executor()
            .advance_clock(crate::table::find::TYPING_DELAY);
        self.settle();
    }

    pub fn keys(&mut self, keys: &str) {
        self.ui.simulate_keystrokes(keys);
        self.settle();
    }

    pub fn click(&mut self, point: Point<Pixels>, modifiers: Modifiers) {
        self.ui.simulate_click(point, modifiers);
        self.settle();
    }

    pub fn cell(&self, row: u64, column: usize) -> Point<Pixels> {
        self.read(|t| {
            t.layout.bounds.origin
                + point(
                    px(t.column_left(column).unwrap() + 10.),
                    px(HEADER_HEIGHT + t.layout.vertical.row_top(row) as f32 + ROW_HEIGHT / 2.),
                )
        })
    }

    pub fn header(&self, column: usize) -> Point<Pixels> {
        self.read(|t| {
            t.layout.bounds.origin
                + point(
                    px(t.column_left(column).unwrap() + 10.),
                    px(HEADER_HEIGHT / 2.),
                )
        })
    }

    pub fn selection(&self) -> Option<Selection> {
        self.read(|t| t.selection)
    }

    pub fn source_rows(&self) -> Vec<u64> {
        self.read(|t| t.data.view().source_rows(0, u32::MAX))
    }

    pub fn clipboard(&self) -> Option<String> {
        self.app.read_from_clipboard()?.text()
    }
}

impl Drop for TableWindow {
    fn drop(&mut self) {
        self.app.quit();
    }
}
