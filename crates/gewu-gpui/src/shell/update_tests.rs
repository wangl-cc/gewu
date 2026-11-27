use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

use gpui_kit::{AppContext, TestAppContext, VisualTestContext, test::TestWindowExt};

use super::{CHECK_INTERVAL, RETRY_INTERVAL, Status, UpdateService, Updater, next_check_after};
use crate::platform::update::Release;

#[test]
fn the_update_window_installs_an_offered_release() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("Gewu Beta.app");
    let executable = bundle.join("Contents/MacOS/Gewu");
    let helper = bundle.join("Contents/Helpers/gewu-update");
    fs::create_dir_all(helper.parent().unwrap()).unwrap();
    fs::write(&helper, "#!/bin/sh\ncase \"$1\" in\ncheck) echo '2.1.0';;\ninstall) test -d \"$2\" && test \"$3\" = '2.1.0' || exit 17; echo \"$3\";;\nesac\n").unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    let release = Release::from_executable(&executable).unwrap();
    let version = release.run(None).unwrap().unwrap();

    let app = TestAppContext::single();
    let updater = app.update(|cx| {
        gpui_kit::init(cx);
        let updater = cx.new(|_| Updater {
            release,
            status: Status::Available(version),
            interactive: true,
            window: None,
            task: None,
            timer: None,
        });
        cx.set_global(UpdateService(updater.clone()));
        Updater::show_window(cx);
        updater
    });
    let window = app.read(|cx| updater.read(cx).window.unwrap());
    let mut ui = VisualTestContext::from_window(window, &app);
    ui.update(|w, cx| w.click("install-update", cx));
    ui.run_until_parked();
    app.read(|cx| {
        assert!(matches!(&updater.read(cx).status, Status::Installed(version) if version.to_string() == "2.1.0"));
        assert_eq!(cx.windows().len(), 1);
    });
    app.quit();
}

#[test]
fn next_check_deadline_accounts_for_elapsed_time_retries_and_clock_changes() {
    let now = 172_800.;
    assert_eq!(next_check_after(now, 0., CHECK_INTERVAL), Duration::ZERO);
    assert_eq!(
        next_check_after(now, now - 60., CHECK_INTERVAL),
        Duration::from_secs(86_340)
    );
    assert_eq!(
        next_check_after(now, now - 60., RETRY_INTERVAL),
        Duration::from_secs(3_540)
    );
    assert_eq!(
        next_check_after(now, now + 60., CHECK_INTERVAL),
        Duration::from_secs(86_400)
    );
}
