use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context as _, Result, bail};
use gewu_gpui::capture::{self, DocumentSession};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Context, HeadlessAppContext, MouseButton, Pixels,
    PlatformTextSystem, Point, Render, Role, Size, Window,
    base::ThemeAppearance,
    div, px, size,
    test::{ElementSnapshot, TestEventExt as _, TestQueryExt as _, TestWindowExt as _},
};
use image::{
    ExtendedColorType, ImageEncoder as _,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};

struct IdleWindow;

impl Render for IdleWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        div()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Mode {
    Standard,
    HighContrast,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::HighContrast => "high-contrast",
        }
    }
}

pub type MenuOpener<'a> = &'a mut dyn FnMut(&mut CaptureSession) -> Result<()>;
pub type FrameVerifier<'a> = &'a mut dyn FnMut(&mut CaptureSession) -> Result<()>;

pub struct CaptureSession {
    app: HeadlessAppContext,
    doc: DocumentSession,
    idle: AnyWindowHandle,
    active_handle: AnyWindowHandle,
    settings: Option<AnyWindowHandle>,
    mode: Mode,
    output: PathBuf,
}

impl CaptureSession {
    pub fn open(
        dataset: &Path,
        window_size: Option<Size<Pixels>>,
        text_system: Arc<dyn PlatformTextSystem>,
        output: PathBuf,
        mode: Mode,
    ) -> Result<Self> {
        let mut app = HeadlessAppContext::with_platform(
            text_system,
            capture::assets(),
            gpui_kit::platform::current_headless_renderer,
        );
        let idle: AnyWindowHandle = app
            .open_window(size(px(1.), px(1.)), |_, cx| cx.new(|_| IdleWindow))?
            .into();
        let doc = app
            .update(|cx| capture::open_document(dataset.to_path_buf(), window_size, cx))
            .context("open document window (requires macOS graphics access)")?;
        let active_handle = doc.window;

        let mut session = Self {
            app,
            doc,
            idle,
            active_handle,
            settings: None,
            mode,
            output,
        };
        session.set_output(ThemeAppearance::Light, true)?;

        let palette = session.update_active(|_, cx| capture::palette_mode(cx))?;
        if mode == Mode::HighContrast || palette != Some("standard") {
            session.open_settings()?;
            let choice = if mode == Mode::HighContrast {
                "On"
            } else {
                "Off"
            };
            session.click_label(choice, Some(Role::RadioButton))?;
            session.focus_document()?;
        }
        Ok(session)
    }

    pub fn elements(&mut self) -> Result<Vec<ElementSnapshot>> {
        self.update_active(|w, _| w.elements())
    }

    pub fn find_by(
        &mut self,
        matcher: impl Fn(&ElementSnapshot) -> bool,
        role: Option<Role>,
    ) -> Result<Option<ElementSnapshot>> {
        let elements = self.elements()?;
        Ok(elements
            .into_iter()
            .find(|el| matcher(el) && role.is_none_or(|r| el.role() == Some(r))))
    }

    pub fn require_by(
        &mut self,
        desc: &str,
        matcher: impl Fn(&ElementSnapshot) -> bool,
        role: Option<Role>,
    ) -> Result<ElementSnapshot> {
        if let Some(found) = self.find_by(matcher, role)? {
            return Ok(found);
        }
        let available: Vec<_> = self
            .elements()?
            .into_iter()
            .filter_map(|el| el.label().or_else(|| el.value()).map(str::to_owned))
            .collect();
        bail!(
            "missing {:?} {desc:?}. Available controls: {}",
            role,
            available.join(", ")
        )
    }

    pub fn require_label(&mut self, label: &str, role: Option<Role>) -> Result<ElementSnapshot> {
        self.require_by(
            label,
            |el| el.label() == Some(label) || el.value() == Some(label),
            role,
        )
    }

    pub fn click_label(&mut self, label: &str, role: Option<Role>) -> Result<()> {
        let center = self.require_label(label, role)?.bounds().center();
        self.click_point(center, MouseButton::Left)
    }

    pub fn click_point(&mut self, pos: Point<Pixels>, button: MouseButton) -> Result<()> {
        self.update_active(|w, cx| {
            w.pointer_move(pos, None, cx);
            w.pointer_down(pos, button, cx);
            w.pointer_up(pos, button, cx);
        })?;
        self.settle()
    }

    pub fn click_cell(&mut self, column: &str, row: u64, button: MouseButton) -> Result<()> {
        let pos = self
            .app
            .update(|cx| capture::cell_point(&self.doc, column, row, cx))
            .with_context(|| format!("missing table cell ({column:?}, {row})"))?;
        self.click_point(pos, button)
    }

    pub fn header_bounds(&self, column: &str) -> Result<Bounds<Pixels>> {
        let app = self.app.app.borrow();
        capture::header_bounds(&self.doc, column, &app)
            .with_context(|| format!("missing table header {column:?}"))
    }

    pub fn click_header(&mut self, column: &str, button: MouseButton) -> Result<()> {
        let center = self.header_bounds(column)?.center();
        self.click_point(center, button)
    }

    pub fn table_dimensions(&self) -> Option<(u64, usize)> {
        let app = self.app.app.borrow();
        capture::table_dimensions(&self.doc, &app)
    }

    pub fn drag(&mut self, from: Point<Pixels>, to: Point<Pixels>) -> Result<()> {
        self.update_active(|w, cx| w.drag(from, to, cx))?;
        self.settle()
    }

    pub fn key(&mut self, keystroke: &str) -> Result<()> {
        self.update_active(|w, cx| w.press(keystroke, cx))?;
        self.settle()
    }

    pub fn input(&mut self, text: &str) -> Result<()> {
        self.update_active(|w, cx| w.input(text, cx))?;
        self.settle()
    }

    pub fn action(&mut self, name: &str) -> Result<()> {
        self.update_active(|w, cx| {
            assert!(
                capture::dispatch_action(name, w, cx),
                "unknown action {name}"
            );
        })?;
        self.settle()
    }

    pub fn open_settings(&mut self) -> Result<()> {
        let before = self.app.update(|cx| cx.windows());
        self.app.update(capture::open_settings);
        let handle = match self.settings {
            Some(handle) => handle,
            None => {
                let handle = self
                    .app
                    .update(|cx| cx.windows().into_iter().find(|h| !before.contains(h)))
                    .context("missing settings window after preferences::open")?;
                self.settings = Some(handle);
                handle
            }
        };
        self.active_handle = handle;
        self.set_output(ThemeAppearance::Light, true)
    }

    pub fn focus_document(&mut self) -> Result<()> {
        self.active_handle = self.doc.window;
        self.set_output(ThemeAppearance::Light, true)
    }

    pub fn capture(
        &mut self,
        code: &str,
        label: &str,
        role: Option<Role>,
        active_only: bool,
        mut menu: Option<MenuOpener<'_>>,
        mut verify: Option<FrameVerifier<'_>>,
    ) -> Result<()> {
        let states: &[(ThemeAppearance, bool)] = if self.mode == Mode::HighContrast || active_only {
            &[
                (ThemeAppearance::Light, true),
                (ThemeAppearance::Dark, true),
            ]
        } else {
            &[
                (ThemeAppearance::Light, true),
                (ThemeAppearance::Light, false),
                (ThemeAppearance::Dark, true),
                (ThemeAppearance::Dark, false),
            ]
        };

        for &(appearance, active) in states {
            let matches_name =
                |el: &ElementSnapshot| el.label() == Some(label) || el.value() == Some(label);
            self.set_output(appearance, active)?;
            if let Some(open_menu) = menu.as_deref_mut()
                && active
                && self.find_by(matches_name, role)?.is_none()
            {
                open_menu(self)?;
            }

            let target_label = if menu.is_some() && !active {
                if self.find_by(matches_name, role)?.is_some() {
                    bail!("menu survived window deactivation");
                }
                "menu dismissed".to_owned()
            } else {
                self.require_label(label, role)?;
                label.to_owned()
            };

            let (palette, is_active, scale, viewport) = self.update_active(|w, cx| {
                (
                    capture::palette_mode(cx),
                    w.is_window_active(),
                    w.scale_factor(),
                    w.viewport_size(),
                )
            })?;
            if palette != Some(self.mode.as_str()) {
                bail!("unexpected installed palette {palette:?}");
            }
            if is_active != active {
                bail!("unexpected active state {is_active} (expected {active})");
            }

            let app_name = match appearance {
                ThemeAppearance::Light => "light",
                ThemeAppearance::Dark => "dark",
            };
            let focus_name = if active { "active" } else { "inactive" };
            let path = self
                .output
                .join(self.mode.as_str())
                .join(format!("{app_name}-{focus_name}-2x"))
                .join(format!("{code}.png"));
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }

            let image = self.app.capture_screenshot(self.active_handle)?;
            let expected = (
                (viewport.width.as_f32() * scale).round() as u32,
                (viewport.height.as_f32() * scale).round() as u32,
            );
            if (image.width(), image.height()) != expected {
                bail!(
                    "unexpected dimensions {}x{} for {} (expected {}x{})",
                    image.width(),
                    image.height(),
                    path.display(),
                    expected.0,
                    expected.1
                );
            }

            let writer = BufWriter::new(fs::File::create(&path)?);
            PngEncoder::new_with_quality(writer, CompressionType::Best, FilterType::Adaptive)
                .write_image(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                    ExtendedColorType::Rgba8,
                )?;

            if let Some(check) = verify.as_deref_mut() {
                check(self)?;
            }
            println!(
                "{code}\t{app_name}\t{focus_name}\t{}\t{}\t{}\t{target_label}",
                scale as u32,
                image.width(),
                image.height()
            );
        }

        self.set_output(ThemeAppearance::Light, true)?;
        if let Some(open_menu) = menu
            && self
                .find_by(
                    |el| el.label() == Some(label) || el.value() == Some(label),
                    role,
                )?
                .is_none()
        {
            open_menu(self)?;
        }
        Ok(())
    }

    fn set_output(&mut self, appearance: ThemeAppearance, active: bool) -> Result<()> {
        let target = if active {
            self.active_handle
        } else {
            self.idle
        };
        self.app
            .update_window(target, |_, w, _| w.activate_window())?;
        self.app.run_until_parked();
        self.app
            .update(|cx| capture::set_appearance(appearance, cx));
        self.settle()
    }

    fn settle(&mut self) -> Result<()> {
        self.app.run_until_parked();
        self.app.advance_clock(Duration::from_millis(280));
        for _ in 0..3 {
            self.app.run_until_parked();
            let handle = self.active_handle;
            self.app
                .update_window(handle, |_, w, cx| w.render_frame(cx))?;
        }
        Ok(())
    }

    pub fn update_active<R>(
        &mut self,
        f: impl FnOnce(&mut Window, &mut gpui_kit::App) -> R,
    ) -> Result<R> {
        let handle = self.active_handle;
        self.app.update_window(handle, |_, w, cx| f(w, cx))
    }
}
