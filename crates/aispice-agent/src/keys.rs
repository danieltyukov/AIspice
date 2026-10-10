//! API key storage.
//!
//! Keys live in the OS keychain (Secret Service on Linux, Keychain on macOS,
//! Credential Manager on Windows) under service `aispice` with the provider
//! id as the user name. When the keychain is missing or failing, as on a
//! headless Linux box with no Secret Service, they fall back to a JSON file
//! that only the user can read. Environment variables win over both so CI
//! and one-off runs never depend on stored keys.
//!
//! Key values are never logged, printed or included in errors.
//!
//! Every call here is blocking (the keychain is reached over D-Bus or a
//! system API). From async code, call it inside `spawn_blocking`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::fsutil::write_atomic;

pub const KEYCHAIN_SERVICE: &str = "aispice";

/// Providers checked by [`KeyStore::list_configured`] even when no backend
/// can enumerate its entries (the keychain cannot).
pub const KNOWN_PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "google",
    "openrouter",
    "ollama",
    "custom",
];

/// Environment variables that supply a provider's key, in priority order.
pub fn env_vars(provider: &str) -> &'static [&'static str] {
    match provider {
        "anthropic" => &["ANTHROPIC_API_KEY"],
        "openai" => &["OPENAI_API_KEY"],
        "google" => &["GOOGLE_API_KEY", "GEMINI_API_KEY"],
        "openrouter" => &["OPENROUTER_API_KEY"],
        _ => &[],
    }
}

/// `<config dir>/aispice/keys.json`, the file fallback.
pub fn default_keys_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("aispice").join("keys.json"))
}

/// `~/.config/aispice/config.json`, where aispice 0.1 kept keys in plain
/// text. That version used this path on every platform.
pub fn legacy_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("aispice").join("config.json"))
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("could not access {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path} is not valid JSON: {message}")]
    Format { path: PathBuf, message: String },
    #[error("the key is empty")]
    Empty,
}

/// A place keys can be kept.
pub trait SecretBackend: Send + Sync {
    /// Short name for messages: `keychain`, `file` or `memory`.
    fn name(&self) -> &'static str;
    fn get(&self, provider: &str) -> Result<Option<String>, KeyError>;
    fn set(&self, provider: &str, key: &str) -> Result<(), KeyError>;
    /// Returns whether a key was there.
    fn remove(&self, provider: &str) -> Result<bool, KeyError>;
    /// Providers with a stored key, when the backend can enumerate them.
    fn list(&self) -> Result<Option<Vec<String>>, KeyError> {
        Ok(None)
    }
}

/// The OS keychain, through the `keyring` crate.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeychainBackend;

impl KeychainBackend {
    fn entry(provider: &str) -> Result<keyring::Entry, KeyError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, provider).map_err(keychain_error)
    }
}

/// Describe a keychain error without echoing stored data, which some
/// variants carry.
fn keychain_error(err: keyring::Error) -> KeyError {
    KeyError::Keychain(match err {
        keyring::Error::BadEncoding(_) => "the stored value is not valid UTF-8".into(),
        keyring::Error::BadDataFormat(..) => "the stored value has an unexpected format".into(),
        other => other.to_string(),
    })
}

impl SecretBackend for KeychainBackend {
    fn name(&self) -> &'static str {
        "keychain"
    }

    fn get(&self, provider: &str) -> Result<Option<String>, KeyError> {
        match Self::entry(provider)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(keychain_error(err)),
        }
    }

    fn set(&self, provider: &str, key: &str) -> Result<(), KeyError> {
        Self::entry(provider)?
            .set_password(key)
            .map_err(keychain_error)
    }

    fn remove(&self, provider: &str) -> Result<bool, KeyError> {
        match Self::entry(provider)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(err) => Err(keychain_error(err)),
        }
    }
}

/// A JSON file, mode 0600 on Unix.
#[derive(Debug, Clone)]
pub struct FileBackend {
    path: PathBuf,
}

#[derive(Default, Serialize, Deserialize)]
struct KeyFile {
    #[serde(default)]
    keys: BTreeMap<String, String>,
}

impl FileBackend {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<KeyFile, KeyError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| KeyError::Format {
                path: self.path.clone(),
                message: e.to_string(),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(KeyFile::default()),
            Err(source) => Err(KeyError::Io {
                path: self.path.clone(),
                source,
            }),
        }
    }

    fn write(&self, file: &KeyFile) -> Result<(), KeyError> {
        let mut text = serde_json::to_string_pretty(file).expect("a string map serializes");
        text.push('\n');
        write_atomic(&self.path, text.as_bytes(), true).map_err(|source| KeyError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

impl SecretBackend for FileBackend {
    fn name(&self) -> &'static str {
        "file"
    }

    fn get(&self, provider: &str) -> Result<Option<String>, KeyError> {
        Ok(self.read()?.keys.remove(provider))
    }

    fn set(&self, provider: &str, key: &str) -> Result<(), KeyError> {
        let mut file = self.read()?;
        file.keys.insert(provider.to_string(), key.to_string());
        self.write(&file)
    }

    fn remove(&self, provider: &str) -> Result<bool, KeyError> {
        let mut file = self.read()?;
        if file.keys.remove(provider).is_none() {
            return Ok(false);
        }
        self.write(&file)?;
        Ok(true)
    }

    fn list(&self) -> Result<Option<Vec<String>>, KeyError> {
        Ok(Some(self.read()?.keys.into_keys().collect()))
    }
}

/// Keys in memory only, for tests and for embedding without persistence.
#[derive(Debug, Default)]
pub struct MemoryBackend {
    keys: Mutex<BTreeMap<String, String>>,
}

impl SecretBackend for MemoryBackend {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn get(&self, provider: &str) -> Result<Option<String>, KeyError> {
        Ok(self.keys.lock().unwrap().get(provider).cloned())
    }

    fn set(&self, provider: &str, key: &str) -> Result<(), KeyError> {
        self.keys
            .lock()
            .unwrap()
            .insert(provider.to_string(), key.to_string());
        Ok(())
    }

    fn remove(&self, provider: &str) -> Result<bool, KeyError> {
        Ok(self.keys.lock().unwrap().remove(provider).is_some())
    }

    fn list(&self) -> Result<Option<Vec<String>>, KeyError> {
        Ok(Some(self.keys.lock().unwrap().keys().cloned().collect()))
    }
}

/// Where a provider's key comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeySource {
    Env { var: String },
    Store { backend: String },
}

impl fmt::Display for KeySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeySource::Env { var } => write!(f, "environment variable {var}"),
            KeySource::Store { backend } if backend == "keychain" => f.write_str("OS keychain"),
            KeySource::Store { backend } if backend == "file" => f.write_str("keys file"),
            KeySource::Store { backend } => write!(f, "{backend} store"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyInfo {
    pub provider: String,
    pub source: KeySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    pub path: PathBuf,
    /// Providers whose keys moved into the store.
    pub migrated: Vec<String>,
    /// Providers that could not be stored, with the reason. Their keys stay
    /// in the legacy file so nothing is lost.
    pub failed: Vec<(String, String)>,
}

type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Reads and writes API keys. Build one with [`KeyStore::system`] for real
/// use and [`KeyStore::memory`] in tests, which never touches the keychain.
pub struct KeyStore {
    primary: Box<dyn SecretBackend>,
    fallback: Option<Box<dyn SecretBackend>>,
    env: Option<EnvLookup>,
}

impl KeyStore {
    /// One backend, no fallback, no environment lookup.
    pub fn new(primary: impl SecretBackend + 'static) -> Self {
        Self {
            primary: Box::new(primary),
            fallback: None,
            env: None,
        }
    }

    /// The OS keychain with the 0600 file as fallback, and environment
    /// variables taking precedence.
    pub fn system() -> Self {
        let store = Self::new(KeychainBackend).with_process_env();
        match default_keys_path() {
            Some(path) => store.with_fallback(FileBackend::new(path)),
            None => store,
        }
    }

    pub fn memory() -> Self {
        Self::new(MemoryBackend::default())
    }

    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::new(FileBackend::new(path))
    }

    pub fn with_fallback(mut self, fallback: impl SecretBackend + 'static) -> Self {
        self.fallback = Some(Box::new(fallback));
        self
    }

    /// Look keys up in the environment first, through `lookup` (handy in
    /// tests, which cannot safely change the real environment).
    pub fn with_env(
        mut self,
        lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.env = Some(Arc::new(lookup));
        self
    }

    pub fn with_process_env(self) -> Self {
        self.with_env(|var| std::env::var(var).ok())
    }

    fn env_key(&self, provider: &str) -> Option<(&'static str, String)> {
        let lookup = self.env.as_ref()?;
        env_vars(provider).iter().find_map(|var| {
            lookup(var)
                .filter(|v| !v.trim().is_empty())
                .map(|v| (*var, v.trim().to_string()))
        })
    }

    fn stored(&self, provider: &str) -> Result<Option<(String, &'static str)>, KeyError> {
        match self.primary.get(provider) {
            Ok(Some(key)) if !key.is_empty() => return Ok(Some((key, self.primary.name()))),
            Ok(_) => {}
            Err(err) if self.fallback.is_some() => {
                tracing::warn!(backend = self.primary.name(), error = %err, "key store unavailable, trying the fallback");
            }
            Err(err) => return Err(err),
        }
        match &self.fallback {
            Some(fallback) => Ok(fallback
                .get(provider)?
                .filter(|k| !k.is_empty())
                .map(|k| (k, fallback.name()))),
            None => Ok(None),
        }
    }

    /// The key for `provider`, from the environment or a store.
    pub fn get(&self, provider: &str) -> Result<Option<String>, KeyError> {
        if let Some((_, key)) = self.env_key(provider) {
            return Ok(Some(key));
        }
        Ok(self.stored(provider)?.map(|(key, _)| key))
    }

    /// Where `provider`'s key would come from, without returning it.
    pub fn source(&self, provider: &str) -> Result<Option<KeySource>, KeyError> {
        if let Some((var, _)) = self.env_key(provider) {
            return Ok(Some(KeySource::Env { var: var.into() }));
        }
        Ok(self.stored(provider)?.map(|(_, backend)| KeySource::Store {
            backend: backend.into(),
        }))
    }

    /// Store a key. Returns where it went. A successful keychain write also
    /// clears any copy in the fallback file so no stale plain-text key stays
    /// behind.
    pub fn set(&self, provider: &str, key: &str) -> Result<KeySource, KeyError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(KeyError::Empty);
        }
        let stored_in = match self.primary.set(provider, key) {
            Ok(()) => {
                if let Some(fallback) = &self.fallback
                    && let Err(err) = fallback.remove(provider)
                {
                    tracing::warn!(backend = fallback.name(), error = %err, "could not clear the fallback copy");
                }
                self.primary.name()
            }
            Err(err) => {
                let Some(fallback) = &self.fallback else {
                    return Err(err);
                };
                tracing::warn!(backend = self.primary.name(), error = %err, "storing the key in the fallback");
                fallback.set(provider, key)?;
                fallback.name()
            }
        };
        Ok(KeySource::Store {
            backend: stored_in.into(),
        })
    }

    /// Delete a stored key from every backend. Returns whether one existed.
    /// Environment variables are left alone.
    pub fn remove(&self, provider: &str) -> Result<bool, KeyError> {
        let primary = self.primary.remove(provider);
        let fallback = self.fallback.as_ref().map(|f| f.remove(provider));
        match (primary, fallback) {
            (Ok(a), None) => Ok(a),
            (Ok(a), Some(Ok(b))) => Ok(a || b),
            (Ok(a), Some(Err(err))) => {
                tracing::warn!(error = %err, "could not clear the fallback copy");
                Ok(a)
            }
            (Err(_), Some(Ok(b))) => Ok(b),
            (Err(err), _) => Err(err),
        }
    }

    /// Providers that have a key, and where it comes from. Never the key.
    pub fn list_configured(&self) -> Vec<KeyInfo> {
        let mut providers: BTreeSet<String> =
            KNOWN_PROVIDERS.iter().map(|p| p.to_string()).collect();
        for backend in std::iter::once(&self.primary).chain(self.fallback.as_ref()) {
            if let Ok(Some(list)) = backend.list() {
                providers.extend(list);
            }
        }
        providers
            .into_iter()
            .filter_map(|provider| match self.source(&provider) {
                Ok(Some(source)) => Some(KeyInfo { provider, source }),
                _ => None,
            })
            .collect()
    }

    /// Move keys out of the aispice 0.1 plain-text config. Returns `None`
    /// when there is nothing to migrate.
    pub fn migrate_legacy(&self) -> Result<Option<MigrationReport>, KeyError> {
        match legacy_config_path() {
            Some(path) => self.migrate_legacy_from(&path),
            None => Ok(None),
        }
    }

    /// Like [`KeyStore::migrate_legacy`] for a given file. Migrated keys are
    /// removed from it; every other field is kept.
    pub fn migrate_legacy_from(&self, path: &Path) -> Result<Option<MigrationReport>, KeyError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(KeyError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let mut doc: Value = serde_json::from_str(&text).map_err(|e| KeyError::Format {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        let Some(Value::Object(keys)) = doc.get("api_keys").cloned() else {
            return Ok(None);
        };
        let mut report = MigrationReport {
            path: path.to_path_buf(),
            migrated: Vec::new(),
            failed: Vec::new(),
        };
        let mut remaining = Map::new();
        for (provider, value) in keys {
            let Some(key) = value.as_str().filter(|k| !k.trim().is_empty()) else {
                continue;
            };
            match self.set(&provider, key) {
                Ok(_) => report.migrated.push(provider),
                Err(err) => {
                    report.failed.push((provider.clone(), err.to_string()));
                    remaining.insert(provider, value);
                }
            }
        }
        if let Value::Object(map) = &mut doc {
            if remaining.is_empty() {
                map.remove("api_keys");
            } else {
                map.insert("api_keys".into(), Value::Object(remaining));
            }
        }
        let mut out = serde_json::to_string_pretty(&doc).expect("JSON value serializes");
        out.push('\n');
        write_atomic(path, out.as_bytes(), true).map_err(|source| KeyError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Some(report))
    }
}

impl fmt::Debug for KeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyStore")
            .field("primary", &self.primary.name())
            .field("fallback", &self.fallback.as_ref().map(|b| b.name()))
            .field("env", &self.env.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A keychain that is always down, like a headless box without a
    /// Secret Service.
    struct BrokenKeychain;

    impl SecretBackend for BrokenKeychain {
        fn name(&self) -> &'static str {
            "keychain"
        }
        fn get(&self, _: &str) -> Result<Option<String>, KeyError> {
            Err(KeyError::Keychain("no secret service".into()))
        }
        fn set(&self, _: &str, _: &str) -> Result<(), KeyError> {
            Err(KeyError::Keychain("no secret service".into()))
        }
        fn remove(&self, _: &str) -> Result<bool, KeyError> {
            Err(KeyError::Keychain("no secret service".into()))
        }
    }

    fn fake_env(
        pairs: &[(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<String> + Send + Sync + 'static {
        let map: BTreeMap<&'static str, &'static str> = pairs.iter().copied().collect();
        move |var| map.get(var).map(|v| v.to_string())
    }

    #[test]
    fn memory_store_round_trip() {
        let store = KeyStore::memory();
        assert_eq!(store.get("anthropic").unwrap(), None);
        let source = store.set("anthropic", "  sk-ant-1  ").unwrap();
        assert_eq!(
            source,
            KeySource::Store {
                backend: "memory".into()
            }
        );
        assert_eq!(store.get("anthropic").unwrap().as_deref(), Some("sk-ant-1"));
        assert!(matches!(store.set("openai", "   "), Err(KeyError::Empty)));
        assert!(store.remove("anthropic").unwrap());
        assert!(!store.remove("anthropic").unwrap());
        assert_eq!(store.get("anthropic").unwrap(), None);
    }

    #[test]
    fn environment_wins_over_stored_keys() {
        let store = KeyStore::memory().with_env(fake_env(&[
            ("ANTHROPIC_API_KEY", "from-env"),
            ("GEMINI_API_KEY", "gemini-env"),
            ("OPENAI_API_KEY", "  "),
        ]));
        store.set("anthropic", "stored").unwrap();
        store.set("openai", "stored-openai").unwrap();
        assert_eq!(store.get("anthropic").unwrap().as_deref(), Some("from-env"));
        assert_eq!(
            store.source("anthropic").unwrap(),
            Some(KeySource::Env {
                var: "ANTHROPIC_API_KEY".into()
            })
        );
        // GOOGLE_API_KEY is unset, so the second variable applies.
        assert_eq!(store.get("google").unwrap().as_deref(), Some("gemini-env"));
        // A blank variable does not hide the stored key.
        assert_eq!(
            store.get("openai").unwrap().as_deref(),
            Some("stored-openai")
        );
    }

    #[test]
    fn file_backend_is_private_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("keys.json");
        let store = KeyStore::file(&path);
        store.set("openrouter", "or-key").unwrap();
        store.set("anthropic", "ant-key").unwrap();
        assert_eq!(store.get("openrouter").unwrap().as_deref(), Some("or-key"));
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            on_disk,
            json!({"keys": {"anthropic": "ant-key", "openrouter": "or-key"}})
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let dir_mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(dir_mode & 0o777, 0o700);
        }
        assert!(store.remove("openrouter").unwrap());
        assert_eq!(store.get("openrouter").unwrap(), None);
        // No temporary files left behind.
        let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn falls_back_to_the_file_when_the_keychain_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        let store = KeyStore::new(BrokenKeychain).with_fallback(FileBackend::new(&path));
        let source = store.set("anthropic", "ant").unwrap();
        assert_eq!(
            source,
            KeySource::Store {
                backend: "file".into()
            }
        );
        assert_eq!(store.get("anthropic").unwrap().as_deref(), Some("ant"));
        assert_eq!(
            store.source("anthropic").unwrap(),
            Some(KeySource::Store {
                backend: "file".into()
            })
        );
        assert!(store.remove("anthropic").unwrap());
        assert_eq!(store.get("anthropic").unwrap(), None);
    }

    #[test]
    fn broken_keychain_without_fallback_is_an_error() {
        let store = KeyStore::new(BrokenKeychain);
        assert!(matches!(store.get("anthropic"), Err(KeyError::Keychain(_))));
        assert!(store.set("anthropic", "k").is_err());
    }

    #[test]
    fn keychain_success_clears_a_stale_file_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        FileBackend::new(&path)
            .set("openai", "old-plaintext")
            .unwrap();
        let store = KeyStore::new(MemoryBackend::default()).with_fallback(FileBackend::new(&path));
        store.set("openai", "new").unwrap();
        assert_eq!(FileBackend::new(&path).get("openai").unwrap(), None);
        assert_eq!(store.get("openai").unwrap().as_deref(), Some("new"));
    }

    #[test]
    fn list_configured_reports_sources_never_values() {
        let store = KeyStore::memory().with_env(fake_env(&[("OPENROUTER_API_KEY", "env-secret")]));
        store.set("anthropic", "mem-secret").unwrap();
        store.set("lmstudio", "local-secret").unwrap();
        let listed = store.list_configured();
        assert_eq!(
            listed,
            vec![
                KeyInfo {
                    provider: "anthropic".into(),
                    source: KeySource::Store {
                        backend: "memory".into()
                    }
                },
                KeyInfo {
                    provider: "lmstudio".into(),
                    source: KeySource::Store {
                        backend: "memory".into()
                    }
                },
                KeyInfo {
                    provider: "openrouter".into(),
                    source: KeySource::Env {
                        var: "OPENROUTER_API_KEY".into()
                    }
                },
            ]
        );
        let shown = format!(
            "{listed:?} {store:?} {}",
            serde_json::to_string(&listed).unwrap()
        );
        assert!(!shown.contains("secret"), "{shown}");
    }

    #[test]
    fn migrates_legacy_plaintext_config() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("config.json");
        std::fs::write(
            &legacy,
            r#"{"api_keys": {"google": "AIza-legacy", "openai": "sk-legacy", "empty": ""}, "theme": "dark", "recent": ["a.asc"]}"#,
        )
        .unwrap();
        let store = KeyStore::memory();
        let report = store.migrate_legacy_from(&legacy).unwrap().unwrap();
        assert_eq!(report.migrated, vec!["google", "openai"]);
        assert!(report.failed.is_empty());
        assert_eq!(store.get("google").unwrap().as_deref(), Some("AIza-legacy"));
        assert_eq!(store.get("openai").unwrap().as_deref(), Some("sk-legacy"));
        let rewritten: Value =
            serde_json::from_str(&std::fs::read_to_string(&legacy).unwrap()).unwrap();
        assert_eq!(rewritten, json!({"theme": "dark", "recent": ["a.asc"]}));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&legacy).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Running it again finds nothing to do.
        assert_eq!(store.migrate_legacy_from(&legacy).unwrap(), None);
    }

    #[test]
    fn failed_migration_keeps_the_key_in_the_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("config.json");
        std::fs::write(&legacy, r#"{"api_keys": {"google": "AIza"}}"#).unwrap();
        let store = KeyStore::new(BrokenKeychain);
        let report = store.migrate_legacy_from(&legacy).unwrap().unwrap();
        assert!(report.migrated.is_empty());
        assert_eq!(report.failed[0].0, "google");
        let rewritten: Value =
            serde_json::from_str(&std::fs::read_to_string(&legacy).unwrap()).unwrap();
        assert_eq!(rewritten, json!({"api_keys": {"google": "AIza"}}));
    }

    #[test]
    fn missing_legacy_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::memory();
        assert_eq!(
            store
                .migrate_legacy_from(&dir.path().join("nope.json"))
                .unwrap(),
            None
        );
    }

    #[test]
    fn corrupt_key_file_is_reported_without_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.json");
        std::fs::write(&path, "{\"keys\": {\"anthropic\": \"sk-secret\"").unwrap();
        let err = KeyStore::file(&path).get("anthropic").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("not valid JSON"));
        assert!(!text.contains("sk-secret"));
    }
}
