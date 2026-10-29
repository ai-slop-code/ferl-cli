//! `ferl`: a small, scriptable client for the ferl link API.
//!
//! Module map: `cli` (grammar), `commands`/`profile_cli`/`wizard` (workflows),
//! `profile`/`config`/`storage`/`origin`/`secret` (local profiles and keys),
//! `input` (validation), `api`/`transport` (HTTP), `list` (pagination), `table`/
//! `report` (output contracts), `clipboard` (helpers), `cancel` (interruption).

mod api;
mod cancel;
mod cli;
mod clipboard;
mod commands;
mod config;
mod error;
mod input;
mod list;
mod origin;
mod profile;
mod profile_cli;
mod report;
mod secret;
mod storage;
mod table;
mod transport;
mod wizard;

use std::cell::RefCell;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use crate::cli::{Action, Invocation, ParseOutcome};
use crate::commands::Env;
use crate::config::Store;
use crate::error::CliError;
use crate::report::{OutputMode, Reporter};

fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn interactive_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal()
}

fn main() -> ExitCode {
    cancel::install();
    let args: Vec<OsString> = std::env::args_os().collect();
    let env_output = env_string("FERL_OUTPUT");
    let json_requested = cli::requests_json(&args, env_output.as_deref());
    match cli::parse(args, env_output) {
        Ok(ParseOutcome::Text(text)) => {
            let _ = io::stdout().lock().write_all(text.as_bytes());
            ExitCode::SUCCESS
        }
        Ok(ParseOutcome::Invocation(invocation)) => match execute(*invocation) {
            Ok(()) => ExitCode::SUCCESS,
            Err(failure) => {
                let (reporter, error) = *failure;
                reporter.error(&error);
                ExitCode::from(error.exit_code())
            }
        },
        Err(error) => {
            let mode = if json_requested {
                OutputMode::Json
            } else {
                OutputMode::Human
            };
            Reporter::new(mode, "ferl", None).error(&error);
            ExitCode::from(error.exit_code())
        }
    }
}

fn execute(invocation: Invocation) -> Result<(), Box<(Reporter, CliError)>> {
    let Invocation {
        config,
        profile,
        output,
        non_interactive,
        limits,
        key,
        action,
        ..
    } = invocation;
    let command = action.command_name();
    let reporter = Reporter::new(output, command, None);
    let store = Store::resolve(config.as_deref(), env_string("FERL_CONFIG").as_deref())
        .map_err(|error| Box::new((reporter.clone(), error)))?;
    let interactive = output == OutputMode::Human && !non_interactive && interactive_terminal();
    let env = Env {
        store,
        reporter: reporter.clone(),
        output,
        interactive,
        limits,
        profile_flag: profile.clone(),
        env_profile: env_string("FERL_PROFILE"),
        key,
        selected: RefCell::new(None),
    };
    let result = match &action {
        Action::Shorten(opts) => commands::shorten(&env, opts),
        Action::List(opts) => commands::list(&env, opts),
        Action::Delete(opts) => commands::delete(&env, opts),
        Action::Profile(opts) => profile_cli::run(&env, opts),
    };
    result.map_err(|error| {
        let mut reporter = reporter;
        reporter.profile = env.selected.borrow().clone();
        Box::new((reporter, error))
    })
}
