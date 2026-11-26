use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[cfg(target_os = "macos")]
const APP_DIR: &str = "cc.loongw.Gewu";
#[cfg(not(target_os = "macos"))]
const APP_DIR: &str = "gewu";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static STATE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppState {
    pub recent_files: Vec<PathBuf>,
    pub last_update_check: f64,
}

impl AppState {
    pub fn load_from(path: &Path) -> Self {
        let mut state: Self = read_json(path).unwrap_or_default();
        if !state.last_update_check.is_finite() || state.last_update_check < 0. {
            state.last_update_check = 0.;
        }
        state
    }
}

pub(crate) fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_DIR).join("settings.json"))
}

pub(crate) fn state_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    let base = if std::env::var_os("FLATPAK_ID").is_some()
        && std::env::var_os("XDG_STATE_HOME").is_none()
    {
        dirs::data_local_dir()
    } else {
        dirs::state_dir().or_else(dirs::data_local_dir)
    };
    #[cfg(not(target_os = "linux"))]
    let base = dirs::state_dir().or_else(dirs::data_local_dir);
    base.map(|dir| dir.join(APP_DIR).join("state.json"))
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Invalid storage path: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|err| err.to_string())?;
    bytes.push(b'\n');

    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("gewu.json");
    let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));

    let outcome = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.flush()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();

    if let Err(err) = outcome {
        let _ = fs::remove_file(&temp);
        return Err(err.to_string());
    }
    Ok(())
}

pub(crate) fn load_state() -> AppState {
    state_path()
        .as_deref()
        .map(AppState::load_from)
        .unwrap_or_default()
}

pub(crate) fn update_state(f: impl FnOnce(&mut AppState)) -> Result<(), String> {
    let Some(path) = state_path() else {
        return Ok(());
    };
    update_state_at(&path, f)
}

pub(crate) fn update_state_at(path: &Path, f: impl FnOnce(&mut AppState)) -> Result<(), String> {
    let _guard = STATE_LOCK
        .lock()
        .map_err(|_| "State storage lock poisoned.".to_owned())?;
    let mut state = AppState::load_from(path);
    f(&mut state);
    write_json(path, &state)
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
