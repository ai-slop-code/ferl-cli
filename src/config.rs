//! Configuration files, profile records, credential binding, and locked
//! snapshots (business spec §§4.1–4.2). The configuration and the companion
//! credential file are each replaced atomically; they are never treated as one
//! transaction.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CliError;
use crate::origin::{Origin, parse_origin, parse_server_origin};
use crate::secret::{ApiKey, KeySource};
use crate::storage::{
    StoreLock, absolutize, ensure_private_dir, lock_store, read_private_file,
    write_private_file_atomic,
};

pub const SCHEMA_VERSION: u32 = 1;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileRecord {
    pub id: String,
    pub server_origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_bundle: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_http_loopback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, ProfileRecord>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        ConfigFile {
            schema_version: SCHEMA_VERSION,
            default_profile: None,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CredentialEntry {
    pub server_origin: String,
    pub api_key: String,
}

impl fmt::Debug for CredentialEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialEntry")
            .field("server_origin", &self.server_origin)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CredentialsFile {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub credentials: BTreeMap<String, CredentialEntry>,
}

/// Paths for one config location and its companion files.
#[derive(Debug, Clone)]
pub struct Store {
    pub config_path: PathBuf,
    pub credentials_path: PathBuf,
    pub lock_path: PathBuf,
}

impl Store {
    /// `--config` > `FERL_CONFIG` > the platform default (business spec §4.1).
    pub fn resolve(explicit: Option<&Path>, env_config: Option<&str>) -> Result<Store, CliError> {
        let config_path = match (explicit, env_config) {
            (Some(path), _) => absolutize(path)?,
            (None, Some(value)) if !value.is_empty() => absolutize(Path::new(value))?,
            _ => default_config_dir()?.join("config.toml"),
        };
        Ok(Store::from_config_path(config_path))
    }

    pub fn from_config_path(config_path: PathBuf) -> Store {
        let file_name = config_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.toml".to_owned());
        let dir = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let credentials_path = dir.join(format!("{file_name}.credentials.toml"));
        let lock_path = dir.join(format!("{file_name}.lock"));
        Store {
            config_path,
            credentials_path,
            lock_path,
        }
    }

    pub fn dir(&self) -> &Path {
        self.config_path.parent().unwrap_or_else(|| Path::new("."))
    }

    pub fn exists_dir(&self) -> bool {
        self.dir().is_dir()
    }

    /// Reads a consistent snapshot under a shared lock (or an exclusive lock when a
    /// write follows). A missing directory is an empty configuration.
    pub fn snapshot(&self, exclusive: bool) -> Result<(Snapshot, Option<StoreLock>), CliError> {
        if !self.exists_dir() {
            return Ok((Snapshot::empty(), None));
        }
        ensure_private_dir(self.dir(), false)?;
        let lock = lock_store(&self.lock_path, exclusive)?;
        let snapshot = Snapshot::read(self)?;
        Ok((snapshot, Some(lock)))
    }

    /// Writes the configuration file atomically.
    pub fn save_config(&self, config: &ConfigFile) -> Result<(), CliError> {
        let text = toml::to_string(config).map_err(|e| CliError::internal(e.to_string()))?;
        write_private_file_atomic(&self.config_path, text.as_bytes())
    }

    /// Writes the credential file atomically, dropping entries that no profile
    /// references. Crash leftovers are therefore removed by the next successful write.
    pub fn save_credentials(
        &self,
        credentials: &CredentialsFile,
        referenced_ids: &[String],
    ) -> Result<(), CliError> {
        let mut pruned = credentials.clone();
        pruned
            .credentials
            .retain(|id, _| referenced_ids.iter().any(|r| r == id));
        let text = toml::to_string(&pruned).map_err(|e| CliError::internal(e.to_string()))?;
        write_private_file_atomic(&self.credentials_path, text.as_bytes())
    }

    /// Creates the directory for a write and takes an exclusive lock.
    pub fn write_snapshot(&self) -> Result<(Snapshot, StoreLock), CliError> {
        ensure_private_dir(self.dir(), true)?;
        let lock = lock_store(&self.lock_path, true)?;
        let snapshot = Snapshot::read(self)?;
        Ok((snapshot, lock))
    }
}

fn default_config_dir() -> Result<PathBuf, CliError> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(xdg);
        if !path.is_absolute() {
            return Err(CliError::config(
                "XDG_CONFIG_HOME is set but is not an absolute path. Fix it or use --config.",
            ));
        }
        return Ok(path.join("ferl"));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| CliError::config("No home directory is available. Use --config PATH."))?;
    Ok(home.join(".config").join("ferl"))
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub config: ConfigFile,
    pub credentials: CredentialsFile,
}

impl Snapshot {
    pub fn empty() -> Snapshot {
        Snapshot {
            config: ConfigFile::default(),
            credentials: CredentialsFile {
                schema_version: SCHEMA_VERSION,
                credentials: BTreeMap::new(),
            },
        }
    }

    fn read(store: &Store) -> Result<Snapshot, CliError> {
        let config = match read_private_file(&store.config_path)? {
            Some(text) => {
                let parsed: ConfigFile = toml::from_str(&text).map_err(|_| {
                    CliError::config(format!(
                        "The configuration file {} is malformed or uses an unsupported schema.",
                        store.config_path.display()
                    ))
                })?;
                if parsed.schema_version != SCHEMA_VERSION {
                    return Err(CliError::config(format!(
                        "The configuration file {} uses unsupported schema version {}.",
                        store.config_path.display(),
                        parsed.schema_version
                    )));
                }
                parsed
            }
            None => ConfigFile::default(),
        };
        let credentials = match read_private_file(&store.credentials_path)? {
            Some(text) => {
                let parsed: CredentialsFile = toml::from_str(&text).map_err(|_| {
                    CliError::config(format!(
                        "The credential file {} is malformed or uses an unsupported schema.",
                        store.credentials_path.display()
                    ))
                })?;
                if parsed.schema_version != SCHEMA_VERSION {
                    return Err(CliError::config(format!(
                        "The credential file {} uses unsupported schema version {}.",
                        store.credentials_path.display(),
                        parsed.schema_version
                    )));
                }
                parsed
            }
            None => CredentialsFile {
                schema_version: SCHEMA_VERSION,
                credentials: BTreeMap::new(),
            },
        };
        let snapshot = Snapshot {
            config,
            credentials,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn validate(&self) -> Result<(), CliError> {
        for (name, record) in &self.config.profiles {
            validate_profile_name(name)?;
            Uuid::parse_str(&record.id).map_err(|_| {
                CliError::config(format!("Profile {name} has an invalid stored identifier."))
            })?;
            parse_origin(&record.server_origin).map_err(|_| {
                CliError::config(format!(
                    "Profile {name} has an invalid stored server origin."
                ))
            })?;
            if let Some(public) = &record.public_origin {
                parse_origin(public).map_err(|_| {
                    CliError::config(format!(
                        "Profile {name} has an invalid stored public origin."
                    ))
                })?;
            }
            if let Some(ca) = &record.ca_bundle {
                if !Path::new(ca).is_absolute() {
                    return Err(CliError::config(format!(
                        "Profile {name} has a CA bundle path that is not absolute."
                    )));
                }
            }
        }
        if let Some(default) = &self.config.default_profile {
            if !self.config.profiles.contains_key(default) {
                return Err(CliError::config(
                    "The configured default profile does not exist. Select a profile with profile use.",
                ));
            }
        }
        Ok(())
    }

    pub fn profile(&self, name: &str) -> Result<&ProfileRecord, CliError> {
        self.config
            .profiles
            .get(name)
            .ok_or_else(|| CliError::config(format!("Profile {name} does not exist.")))
    }

    /// The stored credential bound to this profile, if its origin still matches.
    pub fn credential_for(&self, record: &ProfileRecord) -> Option<&CredentialEntry> {
        let entry = self.credentials.credentials.get(&record.id)?;
        let origin = parse_origin(&record.server_origin).ok()?;
        (entry.server_origin == origin.text()).then_some(entry)
    }

    pub fn is_credential_present(&self, record: &ProfileRecord) -> bool {
        self.credential_for(record).is_some()
    }
}

pub fn validate_profile_name(name: &str) -> Result<(), CliError> {
    let mut chars = name.chars();
    let first_ok = matches!(chars.next(), Some(c) if c.is_ascii_lowercase());
    let rest_ok = chars
        .clone()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if first_ok && rest_ok && name.len() <= 32 {
        Ok(())
    } else {
        Err(CliError::usage(
            "A profile name must match [a-z][a-z0-9_-]{0,31}.",
        ))
    }
}

/// Serializable view with no secret material (business spec §7.2 `ProfileView`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProfileView {
    pub name: String,
    pub id: String,
    pub server_origin: String,
    pub public_origin: String,
    pub ca_bundle: Option<String>,
    pub allow_http_loopback: bool,
    pub is_default: bool,
    pub credential_storage: &'static str,
    pub credential_present: bool,
}

impl ProfileView {
    pub fn from_record(snapshot: &Snapshot, name: &str, record: &ProfileRecord) -> ProfileView {
        let server = record.server_origin.clone();
        let public = record
            .public_origin
            .clone()
            .unwrap_or_else(|| server.clone());
        ProfileView {
            name: name.to_owned(),
            id: record.id.clone(),
            server_origin: server,
            public_origin: public,
            ca_bundle: record.ca_bundle.clone(),
            allow_http_loopback: record.allow_http_loopback,
            is_default: snapshot.config.default_profile.as_deref() == Some(name),
            credential_storage: "private_file",
            credential_present: snapshot.is_credential_present(record),
        }
    }
}

/// Where a business command's key comes from. Stored keys are read only when used.
pub enum KeyRef {
    Override(KeySource),
    Stored(CredentialEntry),
}

/// A profile selected for one invocation: origins, CA setting, and a deferred key.
pub struct ProfileTarget {
    pub name: String,
    pub server: Origin,
    pub public: Origin,
    pub ca_bundle: Option<PathBuf>,
    pub key: KeyRef,
}

impl ProfileTarget {
    /// Reads the key. Called only after local validation and any consent check.
    pub fn read_key(&self) -> Result<ApiKey, CliError> {
        match &self.key {
            KeyRef::Override(source) => source.read(),
            KeyRef::Stored(entry) => ApiKey::from_input(entry.api_key.clone().into_bytes()),
        }
    }
}

impl fmt::Debug for ProfileTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileTarget")
            .field("name", &self.name)
            .field("server", &self.server.text())
            .field("key", &"<deferred>")
            .finish()
    }
}

/// Selects one profile name using `--profile` > `FERL_PROFILE` > the saved default.
pub fn select_profile_name(
    snapshot: &Snapshot,
    explicit: Option<&str>,
    env_profile: Option<&str>,
) -> Result<String, CliError> {
    if let Some(name) = explicit {
        validate_profile_name(name)?;
        return Ok(name.to_owned());
    }
    if let Some(name) = env_profile.filter(|v| !v.is_empty()) {
        validate_profile_name(name)?;
        return Ok(name.to_owned());
    }
    snapshot.config.default_profile.clone().ok_or_else(|| {
        CliError::config(
            "No profile is selected and no default profile is set. Use --profile NAME or run ferl profile.",
        )
    })
}

/// Resolves a profile's origins and CA setting for a business command. The key is
/// not read yet, so a delete can check its target and consent first.
///
/// A key override is in memory only; it never updates the profile. The bound
/// stored credential is used only when its origin matches the profile.
pub fn resolve_profile(
    snapshot: &Snapshot,
    name: &str,
    key_override: Option<&KeySource>,
) -> Result<ProfileTarget, CliError> {
    let record = snapshot.profile(name)?;
    let server = parse_server_origin(&record.server_origin, record.allow_http_loopback)?;
    let public = match &record.public_origin {
        Some(text) => parse_origin(text)?,
        None => server.clone(),
    };
    let key = match key_override {
        Some(source) => KeyRef::Override(source.clone()),
        None => {
            let entry = snapshot.credential_for(record).ok_or_else(|| {
                CliError::config(format!(
                    "Profile {name} has no usable saved key. Run profile set-key {name} or supply a key source."
                ))
            })?;
            KeyRef::Stored(entry.clone())
        }
    };
    Ok(ProfileTarget {
        name: name.to_owned(),
        server,
        public,
        ca_bundle: record.ca_bundle.as_deref().map(PathBuf::from),
        key,
    })
}

pub fn new_profile_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_follow_the_documented_pattern() {
        assert!(validate_profile_name("work").is_ok());
        assert!(validate_profile_name("a-b_c9").is_ok());
        assert!(validate_profile_name("Work").is_err());
        assert!(validate_profile_name("9work").is_err());
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name(&"a".repeat(33)).is_err());
    }

    #[test]
    fn default_config_location_uses_absolute_xdg_or_home() {
        let store = Store::from_config_path(PathBuf::from("/abs/dir/config.toml"));
        assert_eq!(
            store.credentials_path,
            PathBuf::from("/abs/dir/config.toml.credentials.toml")
        );
        assert_eq!(store.lock_path, PathBuf::from("/abs/dir/config.toml.lock"));
    }

    #[test]
    fn selection_prefers_flag_then_env_then_default() {
        let mut snapshot = Snapshot::empty();
        snapshot.config.default_profile = None;
        assert!(select_profile_name(&snapshot, None, None).is_err());
        assert_eq!(
            select_profile_name(&snapshot, Some("a"), Some("b")).expect("flag"),
            "a"
        );
        assert_eq!(
            select_profile_name(&snapshot, None, Some("b")).expect("env"),
            "b"
        );
    }

    #[test]
    fn credential_entries_are_debug_redacted() {
        let entry = CredentialEntry {
            server_origin: "https://api.example.com".to_owned(),
            api_key: "ferl_ak_v1_sentinel".to_owned(),
        };
        assert!(!format!("{entry:?}").contains("sentinel"));
    }
}
