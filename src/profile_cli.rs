//! Dispatch for `ferl profile …`: scripted forms and their JSON/human outputs. Missing
//! values are prompted for only through explicit interactive paths (business spec §3.2).

use std::io::{self, Write};

use serde_json::json;

use crate::cli::{AddOpts, ProfileAction, SetOpts};
use crate::commands::Env;
use crate::config::ProfileView;
use crate::error::{CliError, Outcome};
use crate::profile::{self, AddRequest, ConnectionChanges};
use crate::report::{OutputMode, write_result};
use crate::secret::{ApiKey, KeySource};
use crate::storage::absolutize;
use crate::table::simple_table;
use crate::wizard;

fn io_error(error: io::Error) -> CliError {
    CliError::local_io(format!("Could not write the command output: {error}."))
        .with_outcome(Outcome::NotApplicable)
}

fn emit_result<D: serde::Serialize, M: serde::Serialize>(
    command: &str,
    profile: Option<&str>,
    data: &D,
    meta: &M,
) -> Result<(), CliError> {
    write_result(command, profile, data, meta).map_err(io_error)
}

fn validation_label(validated: bool) -> &'static str {
    if validated { "passed" } else { "not_run" }
}

/// Reads a key from an explicit source, or prompts when interactive, or fails.
fn key_for_write(env: &Env, source: Option<&KeySource>, prompt: &str) -> Result<ApiKey, CliError> {
    match source {
        Some(source) => source.read(),
        None if env.interactive => wizard::prompt_key(prompt),
        None => Err(CliError::usage(
            "A key source is required: --key-stdin, --key-file, or --key-env.",
        )),
    }
}

fn require_name(env: &Env, name: Option<&str>, what: &str) -> Result<String, CliError> {
    match name {
        Some(name) => Ok(name.to_owned()),
        None if env.interactive => wizard::pick_profile(env, what),
        None => Err(CliError::usage(format!(
            "A profile name is required. Use ferl profile {what} NAME (or run interactively)."
        ))),
    }
}

fn record_profile(env: &Env, name: &str) {
    *env.selected.borrow_mut() = Some(name.to_owned());
}

pub fn run(env: &Env, action: &ProfileAction) -> Result<(), CliError> {
    match action {
        ProfileAction::Wizard => {
            if !env.interactive {
                return Err(CliError::usage(
                    "ferl profile requires an interactive terminal. Noninteractive forms: profile list, profile show NAME, profile add NAME --server ORIGIN <key source>, profile use NAME, profile check NAME <key source>, profile remove NAME --yes.",
                ));
            }
            wizard::menu(env)
        }
        ProfileAction::Add(opts) => add(env, opts),
        ProfileAction::List => list(env),
        ProfileAction::Show(name) => show(env, name.as_deref()),
        ProfileAction::Use(name) => use_profile(env, name.as_deref()),
        ProfileAction::Set(opts) => set(env, opts),
        ProfileAction::SetKey { name, validate } => set_key(env, name.as_deref(), *validate),
        ProfileAction::Check(name) => check(env, name.as_deref()),
        ProfileAction::Remove { name, yes } => remove(env, name.as_deref(), *yes),
    }
}

fn add(env: &Env, opts: &AddOpts) -> Result<(), CliError> {
    let name = match &opts.name {
        Some(name) => name.clone(),
        None if env.interactive => wizard::prompt_name(env)?,
        None => return Err(CliError::usage("A profile name is required.")),
    };
    crate::config::validate_profile_name(&name)?;
    record_profile(env, &name);
    let server = match &opts.server {
        Some(server) => server.clone(),
        None if env.interactive => wizard::prompt_server(opts.allow_http_loopback)?,
        None => {
            return Err(CliError::usage(
                "--server ORIGIN is required in non-interactive use.",
            ));
        }
    };
    let key = key_for_write(env, env.key.as_ref(), "API key")?;
    let ca_bundle = opts.ca_bundle.as_deref().map(absolutize).transpose()?;
    profile::ensure_name_available(&env.store, &name)?;
    let validated = if opts.validate {
        let server_origin = crate::origin::parse_server_origin(&server, opts.allow_http_loopback)?;
        let (account, _request_id) =
            profile::validate_key(&server_origin, ca_bundle.as_deref(), &key, env.limits)?;
        if !env.json() {
            env.reporter
                .notice(&format!("Validated key for account {}.", account.username));
        }
        true
    } else {
        false
    };
    let view = profile::add(
        &env.store,
        AddRequest {
            name: name.clone(),
            server,
            public_origin: opts.public_origin.clone(),
            ca_bundle,
            allow_http_loopback: opts.allow_http_loopback,
            make_default: opts.make_default,
            key,
        },
    )?;
    finish_profile_write(env, "profile.add", &view, validated)
}

fn finish_profile_write(
    env: &Env,
    command: &str,
    view: &ProfileView,
    validated: bool,
) -> Result<(), CliError> {
    if env.output == OutputMode::Json {
        emit_result(
            command,
            Some(&view.name),
            &json!({ "profile": view, "validation": validation_label(validated) }),
            &json!({ "request_id": serde_json::Value::Null }),
        )
    } else {
        env.reporter
            .notice(&format!("Saved profile {}.", view.name));
        Ok(())
    }
}

fn list(env: &Env) -> Result<(), CliError> {
    let listing = profile::list(&env.store)?;
    if env.output == OutputMode::Json {
        return emit_result("profile.list", None, &listing, &json!({}));
    }
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
                p.ca_bundle.clone().unwrap_or_else(|| "-".to_owned()),
                if p.allow_http_loopback { "yes" } else { "no" }.to_owned(),
                if p.credential_present {
                    "stored"
                } else {
                    "missing"
                }
                .to_owned(),
            ]
        })
        .collect();
    let table = simple_table(
        &[
            "DEFAULT",
            "NAME",
            "SERVER",
            "PUBLIC",
            "CA BUNDLE",
            "HTTP LOOPBACK",
            "KEY",
        ],
        rows,
    );
    write_stdout(&table)
}

fn write_stdout(text: &str) -> Result<(), CliError> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(text.as_bytes())
        .and_then(|_| stdout.flush())
        .map_err(io_error)
}

fn show(env: &Env, name: Option<&str>) -> Result<(), CliError> {
    let name = match name {
        Some(name) => name.to_owned(),
        None => profile::default_name(&env.store)?,
    };
    let view = profile::view(&env.store, &name)?;
    record_profile(env, &name);
    if env.output == OutputMode::Json {
        return emit_result(
            "profile.show",
            Some(&name),
            &json!({ "profile": view }),
            &json!({}),
        );
    }
    let text = format!(
        "name: {}\nid: {}\nserver: {}\npublic: {}\nca_bundle: {}\nallow_http_loopback: {}\ndefault: {}\ncredential_storage: {}\ncredential_present: {}\n",
        view.name,
        view.id,
        view.server_origin,
        view.public_origin,
        view.ca_bundle.clone().unwrap_or_else(|| "-".to_owned()),
        view.allow_http_loopback,
        view.is_default,
        view.credential_storage,
        view.credential_present,
    );
    write_stdout(&text)
}

fn use_profile(env: &Env, name: Option<&str>) -> Result<(), CliError> {
    let name = require_name(env, name, "use")?;
    let default = profile::use_default(&env.store, &name)?;
    record_profile(env, &default);
    if env.output == OutputMode::Json {
        emit_result(
            "profile.use",
            Some(&default),
            &json!({ "default_profile": default }),
            &json!({}),
        )
    } else {
        env.reporter
            .notice(&format!("Default profile is now {default}."));
        Ok(())
    }
}

fn set(env: &Env, opts: &SetOpts) -> Result<(), CliError> {
    record_profile(env, &opts.name);
    let changes = ConnectionChanges {
        server: opts.server.clone(),
        public_origin: opts.public_origin.clone(),
        ca_bundle: match &opts.ca_bundle {
            None => None,
            Some(None) => Some(None),
            Some(Some(path)) => Some(Some(absolutize(path)?)),
        },
        allow_http_loopback: opts.allow_http_loopback,
    };
    let new_key = if opts.server.is_some() {
        Some(key_for_write(env, env.key.as_ref(), "API key")?)
    } else {
        None
    };
    let view = profile::set_connection(&env.store, &opts.name, &changes, new_key.as_ref())?;
    finish_profile_write(env, "profile.set", &view, false)
}

fn set_key(env: &Env, name: Option<&str>, validate: bool) -> Result<(), CliError> {
    let name = require_name(env, name, "set-key")?;
    record_profile(env, &name);
    let key = key_for_write(env, env.key.as_ref(), "New API key")?;
    let validated = if validate {
        let target = profile::saved_connection(&env.store, &name)?;
        profile::validate_key(&target.0, target.1.as_deref(), &key, env.limits)?;
        true
    } else {
        false
    };
    let view = profile::set_key(&env.store, &name, &key)?;
    finish_profile_write(env, "profile.set-key", &view, validated)
}

fn check(env: &Env, name: Option<&str>) -> Result<(), CliError> {
    let name = require_name(env, name, "check")?;
    record_profile(env, &name);
    let (account, request_id, view) =
        profile::check(&env.store, &name, env.key.as_ref(), env.limits)?;
    if env.output == OutputMode::Json {
        return emit_result(
            "profile.check",
            Some(&view.name),
            &json!({ "account": account }),
            &json!({ "request_id": request_id }),
        );
    }
    let text = format!(
        "profile: {}\naccount: {}\nrole: {}\nenabled: {}\n",
        view.name, account.username, account.role, account.enabled
    );
    write_stdout(&text)
}

fn remove(env: &Env, name: Option<&str>, yes: bool) -> Result<(), CliError> {
    let name = require_name(env, name, "remove")?;
    record_profile(env, &name);
    if !yes {
        if !env.interactive {
            return Err(CliError::usage(
                "Removing a profile requires --yes in non-interactive use. Nothing was changed.",
            ));
        }
        wizard::confirm_remove(&name)?;
    }
    let default = profile::remove(&env.store, &name)?;
    if env.output == OutputMode::Json {
        emit_result(
            "profile.remove",
            Some(&name),
            &json!({ "name": name, "removed": true, "default_profile": default }),
            &json!({}),
        )
    } else {
        env.reporter.notice(&format!(
            "Removed profile {name}. The account and its links were not changed."
        ));
        Ok(())
    }
}
