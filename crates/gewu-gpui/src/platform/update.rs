//! Run the signed bundle's updater off the UI thread. A development build has
//! no release helper, so it never contacts the update service.
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui_kit::{base::Theme, *};
use semver::Version;

use crate::{preferences::Preferences, shell::update_menus, ui::button};

gpui_kit::actions!(gewu, [
    CheckForUpdates,
    ToggleUpdateChecks,
    ToggleAutomaticUpdates
]);

const CHECK_INTERVAL: f64 = 24. * 60. * 60.;
const RETRY_INTERVAL: f64 = 60. * 60.;

#[derive(Clone)]
struct Release {
    bundle: PathBuf,
    helper: PathBuf,
}

impl Release {
    fn from_executable(executable: &Path) -> Option<Self> {
        let macos = executable.parent()?;
        let contents = macos.parent()?;
        let bundle = contents.parent()?;
        if macos.file_name()? != "MacOS"
            || contents.file_name()? != "Contents"
            || bundle.extension()? != "app"
        {
            return None;
        }
        let helper = contents.join("Helpers/gewu-update");
        helper.is_file().then(|| Self {
            bundle: bundle.to_owned(),
            helper,
        })
    }

    fn run(&self, expected: Option<&Version>) -> Result<Option<Version>, String> {
        let mut process = Command::new(&self.helper);
        process
            .arg(if expected.is_some() {
                "install"
            } else {
                "check"
            })
            .arg(&self.bundle);
        if let Some(version) = expected {
            process.arg(version.to_string());
        }
        let output = process.output().map_err(|err| err.to_string())?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(if message.is_empty() {
                format!("Update helper exited with {}", output.status)
            } else {
                message
            });
        }
        let output = std::str::from_utf8(&output.stdout)
            .map_err(|err| err.to_string())?
            .trim();
        if output.is_empty() {
            return Ok(None);
        }
        let version =
            Version::parse(output).map_err(|err| format!("Invalid update version: {err}"))?;
        if !version.pre.is_empty()
            || !version.build.is_empty()
            || expected.is_some_and(|wanted| *wanted != version)
        {
            return Err(format!("Unexpected update version: {version}"));
        }
        Ok(Some(version))
    }
}

struct UpdateService(Entity<Updater>);
impl Global for UpdateService {}

enum Status {
    Idle,
    Checking,
    Current,
    Available(Version),
    Installing,
    Installed(Version),
    Failed(String),
}

struct Updater {
    release: Release,
    status: Status,
    interactive: bool,
    window: Option<AnyWindowHandle>,
    task: Option<Task<()>>,
    timer: Option<Task<()>>,
}

pub(crate) fn init(cx: &mut App) {
    let Some(release) = std::env::current_exe()
        .ok()
        .and_then(|p| Release::from_executable(&p))
    else {
        return;
    };
    let updater = cx.new(|_| Updater {
        release,
        status: Status::Idle,
        interactive: false,
        window: None,
        task: None,
        timer: None,
    });
    cx.set_global(UpdateService(updater.clone()));
    cx.on_action(|_: &CheckForUpdates, cx| {
        let updater = cx.global::<UpdateService>().0.clone();
        updater.update(cx, |this, cx| this.check(true, cx));
    });
    cx.on_action(|_: &ToggleUpdateChecks, cx| {
        Preferences::update(cx, |p| {
            p.automatic_update_checks = !p.automatic_update_checks;
        });
        update_menus(cx);
        let updater = cx.global::<UpdateService>().0.clone();
        updater.update(cx, |this, cx| this.schedule(cx));
    });
    cx.on_action(|_: &ToggleAutomaticUpdates, cx| {
        Preferences::update(cx, |p| {
            p.automatic_update_install = !p.automatic_update_install;
        });
        update_menus(cx);
    });
    updater.update(cx, |this, cx| this.schedule(cx));
}

fn next_check_after(now: f64, last: f64, interval: f64) -> Duration {
    // A future timestamp from a clock adjustment must not postpone checks indefinitely.
    let elapsed = (now - last).max(0.);
    Duration::from_secs_f64((interval - elapsed).clamp(0., interval))
}

pub(crate) fn menu(cx: &App) -> Menu {
    let available = cx.has_global::<UpdateService>();
    let prefs = Preferences::get(cx);
    let enabled = available && prefs.automatic_update_checks;
    Menu::new("Updates").disabled(!available).items([
        MenuItem::action("Check for Updates…", CheckForUpdates),
        MenuItem::separator(),
        MenuItem::action("Automatically Check for Updates", ToggleUpdateChecks).checked(enabled),
        MenuItem::action("Automatically Install Updates", ToggleAutomaticUpdates)
            .disabled(!enabled)
            .checked(available && prefs.automatic_update_install),
    ])
}

impl Updater {
    fn schedule(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        if !Preferences::get(cx).automatic_update_checks
            || matches!(
                self.status,
                Status::Checking | Status::Installing | Status::Installed(_)
            )
        {
            return;
        }
        let interval = if matches!(self.status, Status::Failed(_)) {
            RETRY_INTERVAL
        } else {
            CHECK_INTERVAL
        };
        let delay = next_check_after(clock(), last_check(), interval);
        let timer = cx.background_executor().timer(delay);
        self.timer = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                if Preferences::get(cx).automatic_update_checks {
                    this.check(false, cx);
                }
            });
        }));
    }

    fn show_window(cx: &mut App) {
        let viewer = cx.global::<UpdateService>().0.clone();
        if let Some(handle) = viewer.read(cx).window
            && cx.windows().contains(&handle)
        {
            let _ = handle.update(cx, |_, w, _| w.activate_window());
            return;
        }
        let options = super::sized_window(460., 200., cx);
        match gpui_kit::open_window(options, cx, |w, cx| {
            super::follow_appearance(w, cx);
            w.set_window_title("Gewu Updates");
            viewer.clone()
        }) {
            Ok((handle, _)) => viewer.update(cx, |this, _| this.window = Some(handle)),
            Err(err) => cx.show_system_notification(SystemNotification {
                tag: "gewu-update".into(),
                title: "Gewu Updates".into(),
                body: err.to_string().into(),
                actions: Vec::new(),
            }),
        }
    }

    fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if manual {
            cx.defer(Self::show_window);
        }
        if matches!(
            self.status,
            Status::Checking | Status::Installing | Status::Installed(_)
        ) {
            self.interactive |= manual;
            return;
        }
        self.interactive = manual;
        self.status = Status::Checking;
        self.timer = None;
        remember_check(clock());
        let release = self.release.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { release.run(None) }).await;
            let _ = this.update(cx, |this, cx| {
                this.task = None;
                match result {
                    Ok(None) => this.status = Status::Current,
                    Ok(Some(version)) => {
                        this.status = Status::Available(version);
                        let prefs = Preferences::get(cx);
                        if !this.interactive
                            && prefs.automatic_update_checks
                            && prefs.automatic_update_install
                        {
                            this.install(cx);
                        } else if this.interactive || prefs.automatic_update_checks {
                            cx.defer(Self::show_window);
                        }
                    }
                    Err(message) => this.status = Status::Failed(message),
                }
                this.schedule(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        let Status::Available(version) = &self.status else {
            return;
        };
        let expected = version.clone();
        self.status = Status::Installing;
        self.timer = None;
        let release = self.release.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { release.run(Some(&expected)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.status = match result {
                    Ok(Some(version)) => Status::Installed(version),
                    Ok(None) => Status::Current,
                    Err(message) => Status::Failed(message),
                };
                this.task = None;
                this.schedule(cx);
                cx.defer(Self::show_window);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for Updater {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Theme::global(cx).tokens.colors;
        let message = match &self.status {
            Status::Idle | Status::Checking => "Checking for updates…".into(),
            Status::Current => "Gewu is up to date.".into(),
            Status::Available(version) => format!("Gewu {version} is available."),
            Status::Installing => "Verifying and installing the update…".into(),
            Status::Installed(version) => {
                format!("Gewu {version} is installed. Quit and reopen Gewu to use the new version.")
            }
            Status::Failed(error) => format!("Update failed: {error}"),
        };
        let mut view = div()
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(if matches!(self.status, Status::Failed(_)) {
                crate::errors::inline_warning(message).into_any_element()
            } else {
                div().child(message).into_any_element()
            });
        if matches!(self.status, Status::Available(_)) {
            view = view.child(
                button("install-update", "Install Update", true, cx)
                    .on_click(cx.listener(|this, _, _, cx| this.install(cx))),
            );
        }
        view
    }
}

fn clock() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn last_check() -> f64 {
    super::storage::load_state().last_update_check
}

fn remember_check(now: f64) {
    let _ = super::storage::update_state(|state| state.last_update_check = now);
}

#[cfg(all(test, unix))]
#[path = "update_tests.rs"]
mod tests;
