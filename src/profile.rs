//! Profile operations shared by scripted commands and the interactive wizard. Each
//! mutation takes the exclusive store lock, stages the affected file completely,
//! and commits with an atomic rename (business spec §4.1).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::api::{self, Account};
use crate::config::{
    ConfigFile, CredentialEntry, CredentialsFile, ProfileRecord, ProfileView, Snapshot, Store,
    new_profile_id, resolve_profile, select_profile_name, validate_profile_name,
};
use crate::error::CliError;
use crate::origin::{Origin, parse_origin, parse_server_origin};
use crate::secret::{ApiKey, KeySource};
use crate::storage::absolutize;
use crate::transport::{Limits, Transport};

/// Validates a key against the server with `GET /api/v1/me`. Nothing is persisted.
pub fn validate_key(
    origin: &Origin,
    ca_bundle: Option<&Path>,
    key: &ApiKey,
    limits: Limits,
) -> Result<(Account, Option<String>), CliError> {
    let transport = Transport::new(origin, key.expose(), ca_bundle, limits)?;
    api::get_current_account(&transport)
}

/// Checks a CA bundle's readability and PEM content without any network use.
pub fn validate_ca_bundle(path: &Path) -> Result<(), CliError> {
    crate::transport::validate_ca_bundle(path)
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub name: String,
    pub server: String,
    pub public_origin: Option<String>,
    pub ca_bundle: Option<PathBuf>,
    pub allow_http_loopback: bool,
    pub make_default: bool,
    pub key: ApiKey,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileList {
    pub items: Vec<ProfileView>,
    pub default_profile: Option<String>,
}

/// Lists profiles sorted by name, without any key material.
pub fn list(store: &Store) -> Result<ProfileList, CliError> {
    let (snapshot, _lock) = store.snapshot(false)?;
    let mut items: Vec<ProfileView> = snapshot
        .config
        .profiles
        .iter()
        .map(|(name, record)| ProfileView::from_record(&snapshot, name, record))
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ProfileList {
        items,
        default_profile: snapshot.config.default_profile.clone(),
    })
}

pub fn show(store: &Store, explicit: Option<&str>) -> Result<ProfileView, CliError> {
    let (snapshot, _lock) = store.snapshot(false)?;
    let name = select_profile_name(&snapshot, explicit, None)?;
    let record = snapshot.profile(&name)?;
    Ok(ProfileView::from_record(&snapshot, &name, record))
}

/// The saved default profile name; fails when none is set.
pub fn default_name(store: &Store) -> Result<String, CliError> {
    let (snapshot, _lock) = store.snapshot(false)?;
    select_profile_name(&snapshot, None, None)
}

/// The saved server origin and CA bundle of a profile, for validation before saving.
pub fn saved_connection(store: &Store, name: &str) -> Result<(Origin, Option<PathBuf>), CliError> {
    let (snapshot, _lock) = store.snapshot(false)?;
    let record = snapshot.profile(name)?;
    let origin = parse_server_origin(&record.server_origin, record.allow_http_loopback)?;
    Ok((origin, record.ca_bundle.as_deref().map(PathBuf::from)))
}

pub fn view(store: &Store, name: &str) -> Result<ProfileView, CliError> {
    show(store, Some(name))
}

fn canonical_public(text: Option<&str>) -> Result<Option<String>, CliError> {
    match text {
        Some(raw) => Ok(Some(parse_origin(raw)?.text().to_owned())),
        None => Ok(None),
    }
}

fn absolute_ca(path: Option<&Path>) -> Result<Option<String>, CliError> {
    match path {
        Some(p) => {
            let absolute = absolutize(p)?;
            validate_ca_bundle(&absolute)?;
            Ok(Some(absolute.to_string_lossy().into_owned()))
        }
        None => Ok(None),
    }
}

/// Adds a profile. Never overwrites an existing name and never copies a key from
/// another profile. The credential entry is written before the config reference.
pub fn add(store: &Store, request: AddRequest) -> Result<ProfileView, CliError> {
    validate_profile_name(&request.name)?;
    let server = parse_server_origin(&request.server, request.allow_http_loopback)?;
    let public = canonical_public(request.public_origin.as_deref())?;
    let ca = absolute_ca(request.ca_bundle.as_deref())?;

    let (mut snapshot, _lock) = store.write_snapshot()?;
    if snapshot.config.profiles.contains_key(&request.name) {
        return Err(CliError::usage(format!(
            "Profile {} already exists. Use profile set or profile set-key to change it.",
            request.name
        )));
    }
    let id = new_profile_id();
    let record = ProfileRecord {
        id: id.clone(),
        server_origin: server.text().to_owned(),
        public_origin: public,
        ca_bundle: ca,
        allow_http_loopback: request.allow_http_loopback,
    };
    snapshot.credentials.credentials.insert(
        id.clone(),
        CredentialEntry {
            server_origin: server.text().to_owned(),
            api_key: request.key.expose().to_owned(),
        },
    );
    let mut referenced: Vec<String> = snapshot
        .config
        .profiles
        .values()
        .map(|r| r.id.clone())
        .collect();
    referenced.push(id);
    store.save_credentials(&snapshot.credentials, &referenced)?;

    let mut next = snapshot.config.clone();
    next.profiles.insert(request.name.clone(), record);
    if request.make_default || next.default_profile.is_none() {
        next.default_profile = Some(request.name.clone());
    }
    store.save_config(&next)?;
    Ok(committed_view(&next, &snapshot.credentials, &request.name))
}

/// Replaces only the saved key of one profile. Other settings and the profile
/// identifier are unchanged. The new key is written atomically to the credential file.
pub fn set_key(store: &Store, name: &str, key: &ApiKey) -> Result<ProfileView, CliError> {
    let (mut snapshot, _lock) = store.write_snapshot()?;
    let record = snapshot.profile(name)?.clone();
    snapshot.credentials.credentials.insert(
        record.id.clone(),
        CredentialEntry {
            server_origin: record.server_origin.clone(),
            api_key: key.expose().to_owned(),
        },
    );
    let referenced: Vec<String> = snapshot
        .config
        .profiles
        .values()
        .map(|r| r.id.clone())
        .collect();
    store.save_credentials(&snapshot.credentials, &referenced)?;
    Ok(committed_view(
        &snapshot.config,
        &snapshot.credentials,
        name,
    ))
}

/// Connection edits. A server change requires a new key and starts a new profile
/// identifier; the old credential entry is pruned by the same write.
#[derive(Debug, Clone, Default)]
pub struct ConnectionChanges {
    pub server: Option<String>,
    pub public_origin: Option<Option<String>>,
    pub ca_bundle: Option<Option<PathBuf>>,
    pub allow_http_loopback: Option<bool>,
}

pub fn set_connection(
    store: &Store,
    name: &str,
    changes: &ConnectionChanges,
    new_key: Option<&ApiKey>,
) -> Result<ProfileView, CliError> {
    let (mut snapshot, _lock) = store.write_snapshot()?;
    let current = snapshot.profile(name)?.clone();
    let mut next_config = snapshot.config.clone();
    let record;
    if let Some(server_text) = &changes.server {
        let key = new_key.ok_or_else(|| {
            CliError::usage("Changing the server requires an explicit key source.")
        })?;
        let allow = changes.allow_http_loopback.unwrap_or(false);
        let server = parse_server_origin(server_text, allow)?;
        let public = match &changes.public_origin {
            Some(value) => canonical_public(value.as_deref())?,
            None => None,
        };
        let ca = match &changes.ca_bundle {
            Some(value) => absolute_ca(value.as_deref())?,
            None => None,
        };
        let id = new_profile_id();
        record = ProfileRecord {
            id: id.clone(),
            server_origin: server.text().to_owned(),
            public_origin: public,
            ca_bundle: ca,
            allow_http_loopback: allow,
        };
        snapshot.credentials.credentials.insert(
            id,
            CredentialEntry {
                server_origin: server.text().to_owned(),
                api_key: key.expose().to_owned(),
            },
        );
    } else {
        let mut updated = current.clone();
        if let Some(value) = &changes.public_origin {
            updated.public_origin = canonical_public(value.as_deref())?;
        }
        if let Some(value) = &changes.ca_bundle {
            updated.ca_bundle = absolute_ca(value.as_deref())?;
        }
        if let Some(allow) = changes.allow_http_loopback {
            let origin = parse_origin(&updated.server_origin)?;
            if !allow && !origin.is_https() {
                return Err(CliError::usage(
                    "This profile uses plaintext loopback HTTP; it cannot be disallowed without changing the server.",
                ));
            }
            updated.allow_http_loopback = allow;
        }
        record = updated;
    }
    let referenced: Vec<String> = std::iter::once(record.id.clone())
        .chain(
            snapshot
                .config
                .profiles
                .iter()
                .filter(|(n, _)| n.as_str() != name)
                .map(|(_, r)| r.id.clone()),
        )
        .collect();
    if changes.server.is_some() {
        store.save_credentials(&snapshot.credentials, &referenced)?;
    }
    next_config.profiles.insert(name.to_owned(), record);
    store.save_config(&next_config)?;
    Ok(committed_view(&next_config, &snapshot.credentials, name))
}

/// The view of a just-committed state. Re-reading the store here would wait on the
/// exclusive lock this process still holds.
fn committed_view(config: &ConfigFile, credentials: &CredentialsFile, name: &str) -> ProfileView {
    let snapshot = Snapshot {
        config: config.clone(),
        credentials: credentials.clone(),
    };
    let record = config
        .profiles
        .get(name)
        .cloned()
        .unwrap_or_else(|| ProfileRecord {
            id: String::new(),
            server_origin: String::new(),
            public_origin: None,
            ca_bundle: None,
            allow_http_loopback: false,
        });
    ProfileView::from_record(&snapshot, name, &record)
}

/// Makes `name` the persisted default. No HTTP.
pub fn use_default(store: &Store, name: &str) -> Result<String, CliError> {
    let (snapshot, _lock) = store.write_snapshot()?;
    snapshot.profile(name)?;
    let mut next = snapshot.config.clone();
    next.default_profile = Some(name.to_owned());
    store.save_config(&next)?;
    Ok(name.to_owned())
}

/// Removes config first, then the unreferenced credential entry. Clears the default
/// rather than selecting another profile.
pub fn remove(store: &Store, name: &str) -> Result<Option<String>, CliError> {
    let (snapshot, _lock) = store.write_snapshot()?;
    snapshot.profile(name)?;
    let mut next = snapshot.config.clone();
    next.profiles.remove(name);
    if next.default_profile.as_deref() == Some(name) {
        next.default_profile = None;
    }
    let default = next.default_profile.clone();
    store.save_config(&next)?;
    let referenced: Vec<String> = next.profiles.values().map(|r| r.id.clone()).collect();
    store.save_credentials(&snapshot.credentials, &referenced)?;
    Ok(default)
}

/// Resolves a profile and checks its key against the server, honoring an explicit
/// key override without persisting it. The store lock is not held during HTTP.
pub fn check(
    store: &Store,
    name: &str,
    key_override: Option<&KeySource>,
    limits: Limits,
) -> Result<(Account, Option<String>, ProfileView), CliError> {
    let (target, view) = {
        let (snapshot, _lock) = store.snapshot(false)?;
        let target = resolve_profile(&snapshot, name, key_override)?;
        let record = snapshot.profile(name)?;
        let view = ProfileView::from_record(&snapshot, name, record);
        (target, view)
    };
    let key = target.read_key()?;
    let transport = Transport::new(
        &target.server,
        key.expose(),
        target.ca_bundle.as_deref(),
        limits,
    )?;
    let (account, request_id) = api::get_current_account(&transport)?;
    Ok((account, request_id, view))
}

pub fn ensure_name_available(store: &Store, name: &str) -> Result<(), CliError> {
    let (snapshot, _lock) = store.snapshot(false)?;
    if snapshot.config.profiles.contains_key(name) {
        return Err(CliError::usage(format!("Profile {name} already exists.")));
    }
    Ok(())
}
