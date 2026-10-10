//! The OpenRouter API key lives in the system credential store (Windows Credential
//! Manager, Secret Service on Linux, Keychain on macOS), never in settings.json.
//! The webview only ever sees whether a key is set and its last characters.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use crate::error::AppResult;

const SERVICE: &str = "io.github.ivall.viralflow";
const ACCOUNT: &str = "openrouter-api-key";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Storage {
    /// The system credential store.
    Keyring,
    /// Fallback when the system has no credential store: a file only the user can read.
    File,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub has_key: bool,
    /// Last four characters, to recognise the key in Settings.
    pub hint: String,
    pub storage: Storage,
}

struct Loaded {
    key: String,
    storage: Storage,
}

pub struct KeyStore {
    fallback: PathBuf,
    loaded: Mutex<Option<Loaded>>,
}

impl KeyStore {
    /// Nothing is read here: Secret Service may show an unlock prompt, so the first
    /// access happens on demand from a background thread.
    pub fn new(data_dir: &Path) -> Self {
        KeyStore { fallback: data_dir.join("openrouter.key"), loaded: Mutex::new(None) }
    }

    pub fn key(&self) -> String {
        self.with_loaded(|loaded| loaded.key.clone())
    }

    pub fn status(&self) -> KeyStatus {
        self.with_loaded(status_of)
    }

    /// Stores the key, or removes it when `key` is empty.
    pub fn set(&self, key: &str) -> AppResult<KeyStatus> {
        let key = key.trim().to_string();
        let storage = if key.is_empty() {
            match entry().and_then(|entry| entry.delete_credential()) {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(error) => log::warn!("cannot remove key from the credential store: {error}"),
            }
            remove_file(&self.fallback)?;
            Storage::Keyring
        } else {
            match entry().and_then(|entry| entry.set_password(&key)) {
                Ok(()) => {
                    remove_file(&self.fallback)?;
                    Storage::Keyring
                }
                Err(error) => {
                    log::warn!("credential store unavailable, keeping the key in a private file: {error}");
                    write_private(&self.fallback, &key)?;
                    Storage::File
                }
            }
        };
        let loaded = Loaded { key, storage };
        let status = status_of(&loaded);
        *self.loaded.lock().expect("key lock") = Some(loaded);
        Ok(status)
    }

    fn with_loaded<T>(&self, read: impl FnOnce(&Loaded) -> T) -> T {
        let mut guard = self.loaded.lock().expect("key lock");
        read(guard.get_or_insert_with(|| self.load()))
    }

    fn load(&self) -> Loaded {
        match entry().and_then(|entry| entry.get_password()) {
            Ok(key) => return Loaded { key, storage: Storage::Keyring },
            Err(keyring::Error::NoEntry) => {}
            Err(error) => log::warn!("cannot read the credential store: {error}"),
        }
        match std::fs::read_to_string(&self.fallback) {
            Ok(key) => Loaded { key: key.trim().to_string(), storage: Storage::File },
            Err(_) => Loaded { key: String::new(), storage: Storage::Keyring },
        }
    }
}

fn status_of(loaded: &Loaded) -> KeyStatus {
    let chars: Vec<char> = loaded.key.chars().collect();
    let hint = if chars.len() > 8 { chars[chars.len() - 4..].iter().collect() } else { String::new() };
    KeyStatus { has_key: !loaded.key.is_empty(), hint, storage: loaded.storage }
}

fn entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT)
}

fn remove_file(path: &Path) -> AppResult<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

fn write_private(path: &Path, key: &str) -> AppResult<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(path)?, key.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_shows_only_the_end_of_long_keys() {
        let status = status_of(&Loaded { key: "example-key-0123456789abcdef".into(), storage: Storage::Keyring });
        assert!(status.has_key);
        assert_eq!(status.hint, "cdef");
        assert_eq!(status_of(&Loaded { key: "short".into(), storage: Storage::File }).hint, "");
    }
}
