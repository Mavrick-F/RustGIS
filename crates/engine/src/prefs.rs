//! The desktop app's preferences, `prefs.json` in the settings folder: one JSON object of
//! string values. The desktop reads and writes it; the command line and Python read it for
//! `--use-saved-secrets` / `use_saved_secrets`.

use rustgis_data::secrets::{Reporting, SecretStore};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const FILE: &str = "prefs.json";

static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Reads the desktop preferences. A missing file is normal; unreadable or malformed files
/// return an error without including any preference values in the diagnostic.
pub fn read() -> Result<Map<String, Value>, String> {
    match rustgis_data::dirs::config_dir() {
        Some(dir) => read_at(&dir.join(FILE)),
        None => Ok(Map::new()),
    }
}

fn read_at(path: &Path) -> Result<Map<String, Value>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(format!("could not read preferences: {error}")),
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("invalid preferences JSON at line {}, column {}", error.line(), error.column()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err("preferences must be a JSON object".into()),
    }
}

/// Updates one desktop preference, preserving all others. A malformed file is set aside (see
/// [`set_aside`]) and a new one started: returns where it was put. An unreadable one is never
/// overwritten. The new file is written under a name of its own beside the old one, synced to
/// disk, then renamed over it, so a crash or power cut leaves the old file or the new one, never
/// a partly written one, and two copies of RustGIS saving at once never share a temporary file.
pub fn write(key: &str, value: &str) -> Result<Option<PathBuf>, String> {
    let dir = rustgis_data::dirs::config_dir().ok_or("the settings folder isn't available")?;
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    write_at(&dir.join(FILE), key, value)
}

/// [`write`] to the file at `path` (a test folder in tests): returns where a malformed file went.
fn write_at(path: &Path, key: &str, value: &str) -> Result<Option<PathBuf>, String> {
    let (mut map, set_aside) = match read_at(path) {
        Ok(map) => (map, None),
        Err(_) if path.is_file() && std::fs::read_to_string(path).is_ok() => (Map::new(), Some(set_aside(path)?)),
        Err(error) => return Err(error),
    };
    map.insert(key.to_string(), Value::String(value.to_string()));
    let text = serde_json::to_vec(&map).map_err(|_| "could not serialize preferences")?;
    let dir = path.parent().ok_or("the settings folder isn't available")?;
    std::fs::create_dir_all(dir).and_then(|_| rustgis_data::io::replace_atomic(path, &text)).map_err(|error| match &set_aside {
        Some(backup) => format!("could not save preferences: {error}; malformed preferences were kept at {}", backup.display()),
        None => format!("could not save preferences: {error}"),
    })?;
    Ok(set_aside)
}

/// How many malformed preference files are kept: `prefs.json.bad`, then `prefs.json.bad2` to
/// `prefs.json.bad9`. Once all are taken, saving refuses to replace any recovery copy or the
/// malformed current file; the user can repair or move a copy to make room.
const KEPT_BAD: u32 = 9;

/// Copies malformed preferences to an exclusively created `.bad` file, synced before saving
/// replaces the original. Concurrent callers cannot overwrite each other's recovery files.
/// A failed copy is removed; a failed later save leaves both original and recovery intact.
fn set_aside(path: &Path) -> Result<PathBuf, String> {
    let name = |n: u32| path.with_extension(if n == 1 { "json.bad".to_string() } else { format!("json.bad{n}") });
    let mut source = std::fs::File::open(path).map_err(|error| format!("could not preserve malformed preferences: {error}"))?;
    for target in (1..=KEPT_BAD).map(name) {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut backup = match options.open(&target) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("could not preserve malformed preferences: {error}")),
        };
        if let Err(error) = std::io::copy(&mut source, &mut backup).and_then(|_| backup.sync_all()) {
            drop(backup);
            let _ = std::fs::remove_file(&target);
            return Err(format!("could not preserve malformed preferences: {error}"));
        }
        return Ok(target);
    }
    Err(format!(
        "all {KEPT_BAD} preference recovery files are occupied; repair or move one before saving settings; the malformed preferences were left unchanged at {}",
        path.display()
    ))
}

/// Opts a native caller into the desktop's saved connections and lazy credential store, which
/// also keeps the Earthdata Login token remembered on this computer.
/// Constructing the store and loading connection definitions never reads a credential.
/// Both preference failures and once-only store failures use the caller's diagnostic sink.
/// Returns the same preference snapshot for assistant provider/key selection.
pub fn opt_in(
    s: &mut crate::Session,
    store: impl SecretStore + 'static,
    warn: impl Fn(String) + Send + Sync + 'static,
) -> Map<String, Value> {
    let prefs = read().unwrap_or_else(|error| {
        warn(format!("{error}; going on without saved preferences"));
        Map::new()
    });
    let store = Reporting::new(store, move |error| {
        warn(format!("saved secrets can't be used ({error}); going on without them"));
    });
    let store: Arc<dyn SecretStore> = Arc::new(store);
    crate::render::rustgis_tiles::earthdata::set_secret_store(Some(store.clone()));
    use_saved_secrets(s, &prefs, store);
    prefs
}

/// Gives `s` the desktop's saved connections and their keychain. Connection commands use the
/// desktop's matching rules (same id, kind and address or endpoint; typed passwords and keys
/// win), with a database client's environment password (`PGPASSWORD`…) before the saved one.
/// Changes to the connections aren't written back. Missing preferences never import legacy
/// connections.
pub fn use_saved_secrets(s: &mut crate::Session, prefs: &Map<String, Value>, store: Arc<dyn SecretStore>) {
    if let Some(text) = prefs.get(crate::connections::PREF).and_then(Value::as_str) {
        s.connections = crate::connections::Connections::from_json(text);
    }
    s.database_logins.set_store(Some(store));
    s.database_logins.prefer_environment_passwords();
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn full_recovery_slots_leave_the_malformed_file_and_every_backup_unchanged() {
        let dir = std::env::temp_dir().join(format!("rustgis-preferences-full-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        std::fs::write(&path, "broken preferences").unwrap();
        let mut kept = Vec::new();
        for _ in 0..KEPT_BAD {
            kept.push(set_aside(&path).unwrap());
        }
        assert!(write_at(&path, "theme", "dark").unwrap_err().contains("left unchanged"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken preferences");
        for backup in kept {
            assert_eq!(std::fs::read_to_string(backup).unwrap(), "broken preferences");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preference_updates_preserve_other_values_and_set_malformed_files_aside() {
        let dir = std::env::temp_dir().join(format!("rustgis-preferences-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        assert_eq!(read_at(&path).unwrap(), Map::new());
        assert_eq!(write_at(&path, "connections", "saved definitions").unwrap(), None);
        assert_eq!(write_at(&path, "theme", "dark").unwrap(), None);
        let map = read_at(&path).unwrap();
        assert_eq!(map.get("connections"), Some(&Value::String("saved definitions".into())));
        assert_eq!(map.get("theme"), Some(&Value::String("dark".into())));
        let broken = r#"{"sensitive-pref":"prefs-sentinel", broken}"#;
        std::fs::write(&path, broken).unwrap();
        let error = read_at(&path).unwrap_err();
        assert!(!error.contains("prefs-sentinel"));
        assert_eq!(write_at(&path, "theme", "light").unwrap(), Some(dir.join("prefs.json.bad")));
        assert_eq!(std::fs::read_to_string(dir.join("prefs.json.bad")).unwrap(), broken);
        assert_eq!(read_at(&path).unwrap().get("theme"), Some(&Value::String("light".into())));
        // A file cut short (as a power cut can leave one) is set aside beside the first, not over it.
        std::fs::write(&path, "").unwrap();
        assert_eq!(write_at(&path, "theme", "dark").unwrap(), Some(dir.join("prefs.json.bad2")));
        assert_eq!(std::fs::read_to_string(dir.join("prefs.json.bad")).unwrap(), broken);
        assert_eq!(write_at(&path, "theme", "light").unwrap(), None);
        // No temporary file is left behind.
        let mut names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["prefs.json", "prefs.json.bad", "prefs.json.bad2"]);
        std::fs::write(&path, r#"["prefs-sentinel"]"#).unwrap();
        assert!(!read_at(&path).unwrap_err().contains("prefs-sentinel"));
        assert!(read_at(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
