//! Interactive profile workflows (business spec §3.2). Prompts and menus go to stderr;
//! key entry is hidden; summaries redact the key; cancellation exits 130 without
//! changing local state. Only the explicitly interactive paths use these functions.

use std::io::{self, Write};

use dialoguer::{Confirm, Input, Password, Select};

use crate::commands::Env;
use crate::config::validate_profile_name;
use crate::error::CliError;
use crate::origin::{parse_origin, parse_server_origin};
use crate::profile::{self, AddRequest, ConnectionChanges};
use crate::secret::ApiKey;
use crate::table::simple_table;

fn map_prompt_error(error: dialoguer::Error) -> CliError {
    let dialoguer::Error::IO(io_error) = error;
    if io_error.kind() == io::ErrorKind::Interrupted {
        CliError::interrupted()
    } else {
        CliError::local_io(format!("The prompt could not be completed: {io_error}."))
    }
}

fn stderr_line(text: &str) {
    let _ = writeln!(io::stderr(), "{text}");
}

fn text_prompt(prompt: &str, allow_empty: bool) -> Result<String, CliError> {
    Input::<String>::new()
        .with_prompt(prompt)
        .allow_empty(allow_empty)
        .interact_text()
        .map_err(map_prompt_error)
}

fn confirm(prompt: &str, default: bool) -> Result<bool, CliError> {
    match Confirm::new()
        .with_prompt(prompt)
        .default(default)
        .interact_opt()
    {
        Ok(Some(answer)) => Ok(answer),
        Ok(None) => Err(CliError::declined()),
        Err(error) => Err(map_prompt_error(error)),
    }
}

fn select(prompt: &str, items: &[String]) -> Result<usize, CliError> {
    match Select::new()
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact_opt()
    {
        Ok(Some(index)) => Ok(index),
        Ok(None) => Err(CliError::declined()),
        Err(error) => Err(map_prompt_error(error)),
    }
}

/// Hidden entry of an account key, repeated until it is valid.
pub fn prompt_key(prompt: &str) -> Result<ApiKey, CliError> {
    loop {
        let text = Password::new()
            .with_prompt(prompt)
            .interact()
            .map_err(map_prompt_error)?;
        match ApiKey::from_input(text.into_bytes()) {
            Ok(key) => return Ok(key),
            Err(error) => stderr_line(&format!("{error}")),
        }
    }
}

pub fn prompt_name(env: &Env) -> Result<String, CliError> {
    loop {
        let name = text_prompt("Profile name", false)?;
        if let Err(error) = validate_profile_name(&name) {
            stderr_line(&error.message);
            continue;
        }
        match profile::ensure_name_available(&env.store, &name) {
            Ok(()) => return Ok(name),
            Err(error) => stderr_line(&error.message),
        }
    }
}

/// Prompts for a server origin. Plaintext HTTP is accepted only for a literal
/// loopback address after an explicit confirmation.
pub fn prompt_server(_allow_loopback: bool) -> Result<String, CliError> {
    loop {
        let text = text_prompt("Server origin (for example https://api.example.com)", false)?;
        match parse_origin(&text) {
            Ok(origin) if origin.is_https() => return Ok(text),
            Ok(origin) if origin.is_loopback_literal() => {
                if confirm(
                    "Allow plaintext HTTP to this loopback address for development?",
                    false,
                )? {
                    return Ok(text);
                }
            }
            Ok(_) => stderr_line("Plaintext HTTP is allowed only for a literal loopback address."),
            Err(error) => stderr_line(&error.message),
        }
    }
}

fn prompt_public_origin() -> Result<Option<String>, CliError> {
    loop {
        let text = text_prompt(
            "Public short-link origin (optional, blank for the server origin)",
            true,
        )?;
        if text.is_empty() {
            return Ok(None);
        }
        match parse_origin(&text) {
            Ok(_) => return Ok(Some(text)),
            Err(error) => stderr_line(&error.message),
        }
    }
}

fn prompt_ca_bundle() -> Result<Option<String>, CliError> {
    loop {
        let text = text_prompt(
            "CA bundle path (optional, blank for public roots only)",
            true,
        )?;
        if text.is_empty() {
            return Ok(None);
        }
        match crate::storage::absolutize(std::path::Path::new(&text))
            .and_then(|p| profile::validate_ca_bundle(&p).map(|()| p))
        {
            Ok(_) => return Ok(Some(text)),
            Err(error) => stderr_line(&error.message),
        }
    }
}

/// Validates a candidate key. On failure the user chooses to save unvalidated,
/// re-enter the key, or cancel. Nothing is saved before this choice.
fn validate_or_choose(
    origin: &str,
    allow_http: bool,
    ca: Option<&std::path::Path>,
    key: ApiKey,
    env: &Env,
) -> Result<(ApiKey, bool), CliError> {
    let server = parse_server_origin(origin, allow_http)?;
    let mut key = key;
    loop {
        match profile::validate_key(&server, ca, &key, env.limits) {
            Ok((account, _)) => {
                stderr_line(&format!(
                    "Validated: account {} ({}).",
                    account.username, account.role
                ));
                return Ok((key, true));
            }
            Err(error) => {
                stderr_line(&format!("Validation failed: {}", error.message));
                let options = [
                    "Save without validation".to_owned(),
                    "Enter the key again".to_owned(),
                    "Cancel (no changes)".to_owned(),
                ];
                match select("What would you like to do?", &options)? {
                    0 => return Ok((key, false)),
                    1 => key = prompt_key("API key")?,
                    _ => return Err(CliError::declined()),
                }
            }
        }
    }
}

fn summary(name: &str, server: &str, public: Option<&str>, ca: Option<&str>, default: bool) {
    stderr_line("Profile summary (key redacted):");
    stderr_line(&format!("  name: {name}"));
    stderr_line(&format!("  server: {server}"));
    stderr_line(&format!(
        "  public short-link origin: {}",
        public.unwrap_or("(server origin)")
    ));
    stderr_line(&format!(
        "  CA bundle: {}",
        ca.unwrap_or("(public roots only)")
    ));
    stderr_line("  key: <redacted>");
    stderr_line(&format!(
        "  default: {}",
        if default { "yes" } else { "no" }
    ));
}

pub fn menu(env: &Env) -> Result<(), CliError> {
    loop {
        let listing = profile::list(&env.store)?;
        if listing.items.is_empty() {
            stderr_line("No profiles yet. Let's add one.");
            return add_flow(env);
        }
        let items = [
            "Add profile",
            "Switch default profile",
            "Inspect profiles",
            "Replace saved key",
            "Edit connection settings",
            "Remove profile",
            "Exit",
        ]
        .map(str::to_owned);
        match select("Profiles", &items)? {
            0 => add_flow(env)?,
            1 => switch_flow(env)?,
            2 => inspect_flow(env)?,
            3 => replace_key_flow(env)?,
            4 => edit_flow(env)?,
            5 => remove_flow(env)?,
            _ => return Ok(()),
        }
    }
}

fn add_flow(env: &Env) -> Result<(), CliError> {
    let name = prompt_name(env)?;
    let server = prompt_server(false)?;
    let parsed = parse_origin(&server)?;
    let allow_http = !parsed.is_https();
    let public = prompt_public_origin()?;
    let ca = prompt_ca_bundle()?;
    let ca_path = ca.as_deref().map(std::path::PathBuf::from);
    let key = prompt_key("API key")?;
    let (key, validated) = if confirm("Validate the key now?", true)? {
        validate_or_choose(&server, allow_http, ca_path.as_deref(), key, env)?
    } else {
        (key, false)
    };
    let existing_default = !profile::list(&env.store)?.items.is_empty();
    let make_default = if existing_default {
        confirm("Make this the default profile?", true)?
    } else {
        true
    };
    summary(
        &name,
        &server,
        public.as_deref(),
        ca.as_deref(),
        make_default,
    );
    if !confirm("Save this profile?", true)? {
        return Err(CliError::declined());
    }
    let view = profile::add(
        &env.store,
        AddRequest {
            name: name.clone(),
            server,
            public_origin: public,
            ca_bundle: ca_path,
            allow_http_loopback: allow_http,
            make_default,
            key,
        },
    )?;
    stderr_line(&format!(
        "Saved profile {}{}{}.",
        view.name,
        if view.is_default { " (default)" } else { "" },
        if validated { "" } else { " without validation" }
    ));
    Ok(())
}

pub fn pick_profile(env: &Env, purpose: &str) -> Result<String, CliError> {
    let listing = profile::list(&env.store)?;
    if listing.items.is_empty() {
        return Err(CliError::config(
            "No profiles exist. Run ferl profile to add one.",
        ));
    }
    let items: Vec<String> = listing
        .items
        .iter()
        .map(|p| {
            format!(
                "{}{} — {}",
                p.name,
                if p.is_default { " (default)" } else { "" },
                p.server_origin
            )
        })
        .collect();
    let index = select(&format!("Select the profile to {purpose}"), &items)?;
    Ok(listing.items[index].name.clone())
}

fn switch_flow(env: &Env) -> Result<(), CliError> {
    let name = pick_profile(env, "make the default")?;
    let view = profile::view(&env.store, &name)?;
    stderr_line(&format!(
        "Selected {} at {}.",
        view.name, view.server_origin
    ));
    if view.is_default {
        stderr_line("It is already the default profile.");
        return Ok(());
    }
    if confirm("Make it the default profile?", true)? {
        profile::use_default(&env.store, &name)?;
        stderr_line(&format!("Default profile is now {name}."));
    }
    Ok(())
}

fn inspect_flow(env: &Env) -> Result<(), CliError> {
    let listing = profile::list(&env.store)?;
    let rows: Vec<Vec<String>> = listing
        .items
        .iter()
        .map(|p| {
            vec![
                if p.is_default {
                    "*".to_owned()
                } else {
                    String::new()
                },
                p.name.clone(),
                p.server_origin.clone(),
                p.public_origin.clone(),
                if p.credential_present {
                    "stored"
                } else {
                    "missing"
                }
                .to_owned(),
            ]
        })
        .collect();
    let _ = io::stderr()
        .write_all(simple_table(&["DEFAULT", "NAME", "SERVER", "PUBLIC", "KEY"], rows).as_bytes());
    Ok(())
}

fn replace_key_flow(env: &Env) -> Result<(), CliError> {
    let name = pick_profile(env, "replace the key for")?;
    let (origin, ca) = profile::saved_connection(&env.store, &name)?;
    let key = prompt_key("New API key")?;
    let ca_path = ca.as_deref();
    let (key, validated) = if confirm("Validate the new key now?", true)? {
        let (checked, _) =
            validate_or_choose(origin.text(), !origin.is_https(), ca_path, key, env)?;
        (checked, true)
    } else {
        (key, false)
    };
    profile::set_key(&env.store, &name, &key)?;
    stderr_line(&format!(
        "Replaced the saved key for {name}{}.",
        if validated { "" } else { " without validation" }
    ));
    Ok(())
}

fn edit_flow(env: &Env) -> Result<(), CliError> {
    let name = pick_profile(env, "edit")?;
    let options = [
        "Server origin (requires a new key; resets public origin, CA bundle, and loopback setting)"
            .to_owned(),
        "Public short-link origin".to_owned(),
        "CA bundle".to_owned(),
        "Plaintext loopback HTTP permission".to_owned(),
        "Cancel".to_owned(),
    ];
    match select("Connection setting to change", &options)? {
        0 => {
            let server = prompt_server(false)?;
            let allow_http = !parse_origin(&server)?.is_https();
            let key = prompt_key("New API key for the new server")?;
            let (key, _) = if confirm("Validate the key now?", true)? {
                validate_or_choose(&server, allow_http, None, key, env)?
            } else {
                (key, false)
            };
            let changes = ConnectionChanges {
                server: Some(server),
                public_origin: Some(None),
                ca_bundle: Some(None),
                allow_http_loopback: Some(allow_http),
            };
            profile::set_connection(&env.store, &name, &changes, Some(&key))?;
            stderr_line(&format!("Updated profile {name}."));
        }
        1 => {
            let public = prompt_public_origin()?;
            let changes = ConnectionChanges {
                public_origin: Some(public),
                ..ConnectionChanges::default()
            };
            profile::set_connection(&env.store, &name, &changes, None)?;
            stderr_line(&format!("Updated profile {name}."));
        }
        2 => {
            let ca = prompt_ca_bundle()?;
            let changes = ConnectionChanges {
                ca_bundle: Some(ca.map(std::path::PathBuf::from)),
                ..ConnectionChanges::default()
            };
            profile::set_connection(&env.store, &name, &changes, None)?;
            stderr_line(&format!("Updated profile {name}."));
        }
        3 => {
            let allow = confirm("Allow plaintext loopback HTTP for this profile?", false)?;
            let changes = ConnectionChanges {
                allow_http_loopback: Some(allow),
                ..ConnectionChanges::default()
            };
            profile::set_connection(&env.store, &name, &changes, None)?;
            stderr_line(&format!("Updated profile {name}."));
        }
        _ => return Err(CliError::declined()),
    }
    Ok(())
}

fn remove_flow(env: &Env) -> Result<(), CliError> {
    let name = pick_profile(env, "remove")?;
    confirm_remove(&name)?;
    profile::remove(&env.store, &name)?;
    stderr_line(&format!(
        "Removed profile {name}. The account and its links were not changed."
    ));
    Ok(())
}

/// Default-No consent before removing the local profile and its key.
pub fn confirm_remove(name: &str) -> Result<(), CliError> {
    let prompt = format!(
        "Remove profile {name} and its local key? The server account and its links are not changed."
    );
    if confirm(&prompt, false)? {
        Ok(())
    } else {
        Err(CliError::declined())
    }
}
