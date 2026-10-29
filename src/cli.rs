//! Command grammar (business spec §3). Clap parses syntax; this module applies the
//! cross-option rules so that every invalid combination fails locally with exit 2
//! before any network, file, or prompt work.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand};

use crate::error::CliError;
use crate::report::OutputMode;
use crate::secret::KeySource;
use crate::table::{Column, parse_columns};
use crate::transport::{
    DEFAULT_CONNECT_TIMEOUT, DEFAULT_OPERATION_TIMEOUT, DEFAULT_READ_RETRIES, Limits,
};

#[derive(Parser, Debug)]
#[command(
    name = "ferl",
    version,
    about = "Shorten URLs, find your links, and delete them.",
    long_about = None,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(flatten)]
    pub shorten: ShortenArgs,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct GlobalArgs {
    /// Configuration file path (overrides FERL_CONFIG and the default location).
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// Profile for a business command (overrides FERL_PROFILE and the default).
    #[arg(long, global = true, value_name = "NAME")]
    pub profile: Option<String>,
    /// Output format: human or json (overrides FERL_OUTPUT).
    #[arg(long, global = true, value_name = "FORMAT")]
    pub output: Option<String>,
    /// Never prompt; required values must be supplied as arguments.
    #[arg(long, global = true)]
    pub non_interactive: bool,
    /// Accepted for compatibility; output is never colored.
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Whole-command HTTP deadline in seconds (0.1 to 3600).
    #[arg(long, global = true, value_name = "SECONDS", value_parser = parse_seconds)]
    pub timeout: Option<Duration>,
    /// Connection establishment deadline in seconds (0.1 to 3600).
    #[arg(long, global = true, value_name = "SECONDS", value_parser = parse_seconds)]
    pub connect_timeout: Option<Duration>,
    /// Retries for transient read failures (0 to 2).
    #[arg(long, global = true, value_name = "COUNT", value_parser = clap::value_parser!(u8).range(0..=2))]
    pub read_retries: Option<u8>,
    /// Read the account key from one line on standard input.
    #[arg(long, global = true)]
    pub key_stdin: bool,
    /// Read the account key from an owner-only file.
    #[arg(long, global = true, value_name = "PATH")]
    pub key_file: Option<PathBuf>,
    /// Read the account key from the named environment variable.
    #[arg(long, global = true, value_name = "NAME")]
    pub key_env: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ShortenArgs {
    /// Optional plain-text title (1 to 200 characters after trimming).
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
    /// Tag to attach; repeat for multiple tags.
    #[arg(long = "tag", value_name = "TEXT", action = ArgAction::Append)]
    pub tags: Vec<String>,
    /// Copy the returned short URL to the clipboard (scripts and JSON).
    #[arg(long, conflicts_with = "no_copy")]
    pub copy: bool,
    /// Do not copy the returned short URL to the clipboard.
    #[arg(long = "no-copy", conflicts_with = "copy")]
    pub no_copy: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a short link for a destination URL.
    Create(CreateArgs),
    /// List your own links, optionally searching their text.
    List(ListArgs),
    /// Delete one link by its short code or its short URL.
    Delete(DeleteArgs),
    /// Manage named server/account profiles.
    Profile(ProfileArgs),
    /// URL shorthand (rewritten to `create` before the final parse).
    #[command(external_subcommand)]
    Shorthand(Vec<OsString>),
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    #[arg(value_name = "DESTINATION")]
    pub destination: String,
    #[command(flatten)]
    pub shorten: ShortenArgs,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Literal text to search in destination, short code, or title.
    #[arg(long, value_name = "TEXT")]
    pub query: Option<String>,
    /// Links per page (1 to 200).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..=200))]
    pub limit: Option<u16>,
    /// Continue from an opaque cursor token.
    #[arg(long, value_name = "TOKEN", conflicts_with = "cursor_file")]
    pub cursor: Option<String>,
    /// Continue from a cursor saved as a JSON string (or null).
    #[arg(long, value_name = "PATH")]
    pub cursor_file: Option<PathBuf>,
    /// Save the continuation token for the next page to a file.
    #[arg(long, value_name = "PATH")]
    pub save_cursor: Option<PathBuf>,
    /// Fetch every remaining page (bounded).
    #[arg(long)]
    pub all: bool,
    /// Maximum pages with --all (1 to 10000, default 100).
    #[arg(long, value_name = "N", requires = "all", value_parser = clap::value_parser!(u32).range(1..=10000))]
    pub max_pages: Option<u32>,
    /// Human output columns, comma-separated, in display order.
    #[arg(long, value_name = "COLUMNS")]
    pub columns: Option<String>,
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// Exact short code, or its short URL on the profile's public origin.
    #[arg(value_name = "CODE-OR-SHORT-URL")]
    pub target: String,
    /// Delete without asking (required for non-interactive use).
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub action: Option<ProfileCommand>,
}

#[derive(Subcommand, Debug)]
pub enum ProfileCommand {
    /// Add a named profile.
    Add(ProfileAddArgs),
    /// List profiles without revealing keys.
    List,
    /// Show one profile (default when omitted).
    Show(ProfileNameOpt),
    /// Select the default profile.
    Use(ProfileNameOpt),
    /// Change a profile's connection settings.
    Set(ProfileSetArgs),
    /// Replace the locally saved key for a profile.
    SetKey(ProfileSetKeyArgs),
    /// Verify a profile's key against the server.
    Check(ProfileNameOpt),
    /// Remove a profile and its local key.
    Remove(ProfileRemoveArgs),
}

#[derive(Args, Debug)]
pub struct ProfileNameOpt {
    pub name: Option<String>,
}

#[derive(Args, Debug)]
pub struct ProfileAddArgs {
    pub name: Option<String>,
    /// Server origin, for example https://api.example.com.
    #[arg(long, value_name = "ORIGIN")]
    pub server: Option<String>,
    /// Public origin used for short URLs, when it differs from the server.
    #[arg(long, value_name = "ORIGIN")]
    pub public_origin: Option<String>,
    /// PEM CA bundle that adds private trust anchors.
    #[arg(long, value_name = "PATH")]
    pub ca_bundle: Option<PathBuf>,
    /// Allow plaintext HTTP to a literal loopback address.
    #[arg(long)]
    pub allow_http_loopback: bool,
    /// Make this the default profile.
    #[arg(long = "default")]
    pub make_default: bool,
    /// Validate the key against the server before saving.
    #[arg(long)]
    pub validate: bool,
}

#[derive(Args, Debug)]
pub struct ProfileSetArgs {
    pub name: String,
    #[arg(long, value_name = "ORIGIN")]
    pub server: Option<String>,
    #[arg(long, value_name = "ORIGIN", conflicts_with = "clear_public_origin")]
    pub public_origin: Option<String>,
    #[arg(long)]
    pub clear_public_origin: bool,
    #[arg(long, value_name = "PATH", conflicts_with = "clear_ca_bundle")]
    pub ca_bundle: Option<PathBuf>,
    #[arg(long)]
    pub clear_ca_bundle: bool,
    #[arg(long, conflicts_with = "disallow_http_loopback")]
    pub allow_http_loopback: bool,
    #[arg(long)]
    pub disallow_http_loopback: bool,
}

#[derive(Args, Debug)]
pub struct ProfileSetKeyArgs {
    pub name: Option<String>,
    /// Validate the new key against the server before saving.
    #[arg(long)]
    pub validate: bool,
}

#[derive(Args, Debug)]
pub struct ProfileRemoveArgs {
    pub name: Option<String>,
    /// Remove without asking (required for non-interactive use).
    #[arg(long)]
    pub yes: bool,
}

fn parse_seconds(text: &str) -> Result<Duration, String> {
    let value: f64 = text
        .parse()
        .map_err(|_| "expected decimal seconds from 0.1 to 3600".to_owned())?;
    if !value.is_finite() || !(0.1..=3600.0).contains(&value) {
        return Err("expected decimal seconds from 0.1 to 3600".to_owned());
    }
    Ok(Duration::from_secs_f64(value))
}

/// Copy behaviour for creation. `Auto` means: copy only in interactive human mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyChoice {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone)]
pub struct ShortenOpts {
    pub destination: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub copy: CopyChoice,
}

#[derive(Debug, Clone)]
pub enum CursorInput {
    Literal(String),
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ListOpts {
    pub query: Option<String>,
    pub limit: u16,
    pub cursor: Option<CursorInput>,
    pub save_cursor: Option<PathBuf>,
    pub all: bool,
    pub max_pages: u32,
    pub columns: Option<Vec<Column>>,
}

#[derive(Debug, Clone)]
pub struct DeleteOpts {
    pub target: String,
    pub yes: bool,
}

#[derive(Debug, Clone)]
pub struct AddOpts {
    pub name: Option<String>,
    pub server: Option<String>,
    pub public_origin: Option<String>,
    pub ca_bundle: Option<PathBuf>,
    pub allow_http_loopback: bool,
    pub make_default: bool,
    pub validate: bool,
}

#[derive(Debug, Clone)]
pub struct SetOpts {
    pub name: String,
    pub server: Option<String>,
    pub public_origin: Option<Option<String>>,
    pub ca_bundle: Option<Option<PathBuf>>,
    pub allow_http_loopback: Option<bool>,
}

#[derive(Debug, Clone)]
pub enum ProfileAction {
    Wizard,
    Add(AddOpts),
    List,
    Show(Option<String>),
    Use(Option<String>),
    Set(SetOpts),
    SetKey {
        name: Option<String>,
        validate: bool,
    },
    Check(Option<String>),
    Remove {
        name: Option<String>,
        yes: bool,
    },
}

#[derive(Debug, Clone)]
pub enum Action {
    Shorten(ShortenOpts),
    List(ListOpts),
    Delete(DeleteOpts),
    Profile(ProfileAction),
}

impl Action {
    /// Command name used in JSON Result objects and diagnostics.
    pub fn command_name(&self) -> &'static str {
        match self {
            Action::Shorten(_) => "create",
            Action::List(_) => "list",
            Action::Delete(_) => "delete",
            Action::Profile(p) => match p {
                ProfileAction::Wizard => "profile",
                ProfileAction::Add(_) => "profile.add",
                ProfileAction::List => "profile.list",
                ProfileAction::Show(_) => "profile.show",
                ProfileAction::Use(_) => "profile.use",
                ProfileAction::Set(_) => "profile.set",
                ProfileAction::SetKey { .. } => "profile.set-key",
                ProfileAction::Check(_) => "profile.check",
                ProfileAction::Remove { .. } => "profile.remove",
            },
        }
    }

    pub fn is_profile(&self) -> bool {
        matches!(self, Action::Profile(_))
    }

    /// Whether the command can perform HTTP (and therefore accepts transport flags).
    fn may_use_http(&self) -> bool {
        match self {
            Action::Shorten(_) | Action::List(_) | Action::Delete(_) => true,
            Action::Profile(ProfileAction::Check(_)) | Action::Profile(ProfileAction::Wizard) => {
                true
            }
            Action::Profile(ProfileAction::Add(opts)) => opts.validate,
            Action::Profile(ProfileAction::SetKey { validate, .. }) => *validate,
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Invocation {
    pub config: Option<PathBuf>,
    pub profile: Option<String>,
    pub output: OutputMode,
    pub non_interactive: bool,
    pub limits: Limits,
    pub key: Option<KeySource>,
    pub action: Action,
}

#[derive(Debug)]
pub enum ParseOutcome {
    /// Help or version text should be printed to stdout with exit 0.
    Text(String),
    Invocation(Box<Invocation>),
}

/// True when the raw arguments request JSON output (for usage diagnostics).
pub fn requests_json(args: &[OsString], env_output: Option<&str>) -> bool {
    let mut iter = args.iter().map(|a| a.to_string_lossy().into_owned());
    let mut explicit = None;
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--output=") {
            explicit = Some(value.to_owned());
        } else if arg == "--output" {
            explicit = iter.next();
        }
    }
    explicit.as_deref().or(env_output) == Some("json")
}

pub fn parse(args: Vec<OsString>, env_output: Option<String>) -> Result<ParseOutcome, CliError> {
    if args.len() <= 1 {
        let mut help = Cli::command().render_help().to_string();
        help.push('\n');
        return Ok(ParseOutcome::Text(help));
    }
    let cli = match parse_argv(&args)? {
        Parsed::Text(text) => return Ok(ParseOutcome::Text(text)),
        Parsed::Cli(cli) => *cli,
    };
    let cli = match &cli.command {
        Some(Command::Shorthand(words)) => match parse_argv(&shorthand_argv(&args, words)?)? {
            Parsed::Text(text) => return Ok(ParseOutcome::Text(text)),
            Parsed::Cli(cli) => *cli,
        },
        _ => cli,
    };
    build_invocation(cli, env_output).map(|inv| ParseOutcome::Invocation(Box::new(inv)))
}

enum Parsed {
    Cli(Box<Cli>),
    Text(String),
}

/// Parses one argv. Help and version requests become text; other errors are usage.
fn parse_argv(args: &[OsString]) -> Result<Parsed, CliError> {
    match Cli::try_parse_from(args) {
        Ok(cli) => Ok(Parsed::Cli(Box::new(cli))),
        Err(error) => match error.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                Ok(Parsed::Text(error.to_string()))
            }
            _ => Err(CliError::usage(first_line(&error.to_string()))),
        },
    }
}

/// `ferl <url> …` becomes `ferl create <url> …` in the original argument order. Any
/// other unknown word is an error, never a destination or inferred command.
fn shorthand_argv(args: &[OsString], words: &[OsString]) -> Result<Vec<OsString>, CliError> {
    let first = words
        .first()
        .map(|w| w.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !is_url_word(&first) {
        return Err(CliError::usage(format!(
            "Unknown command or destination {}. Use ferl --help. A destination must start with http:// or https://.",
            crate::report::sanitize(&first)
        )));
    }
    let index = args.len() - words.len();
    let mut rewritten: Vec<OsString> = args[..index].to_vec();
    rewritten.push(OsString::from("create"));
    rewritten.extend(args[index..].iter().cloned());
    Ok(rewritten)
}

fn is_url_word(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("invalid arguments")
        .trim()
        .trim_start_matches("error:")
        .trim()
        .to_owned()
}

fn build_invocation(cli: Cli, env_output: Option<String>) -> Result<Invocation, CliError> {
    let Cli {
        global,
        shorten,
        command,
    } = cli;

    let action = match command {
        Some(Command::Create(args)) => {
            let mut merged = args.shorten;
            merge_shorten_absent(&mut merged, &shorten)?;
            Action::Shorten(shorten_opts(args.destination, merged)?)
        }
        Some(Command::List(args)) => {
            reject_shorten_args(&shorten)?;
            Action::List(list_opts(args)?)
        }
        Some(Command::Delete(args)) => {
            reject_shorten_args(&shorten)?;
            if args.target.is_empty() {
                return Err(CliError::usage("A short code or short URL is required."));
            }
            Action::Delete(DeleteOpts {
                target: args.target,
                yes: args.yes,
            })
        }
        Some(Command::Profile(args)) => {
            reject_shorten_args(&shorten)?;
            Action::Profile(profile_action(args)?)
        }
        Some(Command::Shorthand(_)) => {
            return Err(CliError::internal("shorthand was not rewritten"));
        }
        None => {
            if shorten.title.is_none()
                && shorten.tags.is_empty()
                && !shorten.copy
                && !shorten.no_copy
            {
                return Err(CliError::usage(
                    "A command or destination is required. Run ferl --help.",
                ));
            }
            return Err(CliError::usage(
                "A destination URL is required before --title, --tag, --copy, or --no-copy.",
            ));
        }
    };

    let output_text = global.output.clone().or(env_output);
    let output = match output_text.as_deref() {
        None | Some("human") => OutputMode::Human,
        Some("json") => OutputMode::Json,
        Some(_) => return Err(CliError::usage("The output format must be human or json.")),
    };

    if action.is_profile() && global.profile.is_some() {
        return Err(CliError::usage(
            "Profile commands name their profile directly and do not accept --profile.",
        ));
    }

    let key = key_source(&global)?;
    let transport_flags = global.timeout.is_some()
        || global.connect_timeout.is_some()
        || global.read_retries.is_some();
    if transport_flags && !action.may_use_http() {
        return Err(CliError::usage(
            "Transport flags apply only to commands that perform HTTP.",
        ));
    }
    if let (Some(timeout), Some(connect)) = (global.timeout, global.connect_timeout) {
        if connect > timeout {
            return Err(CliError::usage(
                "--connect-timeout must not exceed --timeout.",
            ));
        }
    }
    if key.is_some() {
        match &action {
            Action::Profile(ProfileAction::Add(_))
            | Action::Profile(ProfileAction::SetKey { .. })
            | Action::Profile(ProfileAction::Check(_))
            | Action::Profile(ProfileAction::Set(_)) => {}
            Action::Shorten(_) | Action::List(_) | Action::Delete(_) => {
                if global.profile.is_none() {
                    return Err(CliError::usage(
                        "A key override for a business command requires an explicit --profile NAME.",
                    ));
                }
            }
            _ => {
                return Err(CliError::usage(
                    "A key source does not apply to this command.",
                ));
            }
        }
    }
    if let Action::List(list) = &action {
        if list.columns.is_some() && output == OutputMode::Json {
            return Err(CliError::usage(
                "--columns applies only to human list output.",
            ));
        }
    }
    if let Action::Profile(ProfileAction::Set(set)) = &action {
        if set.server.is_none() && key.is_some() {
            return Err(CliError::usage(
                "A key source is supplied only with --server.",
            ));
        }
        if set.server.is_some() && key.is_none() {
            return Err(CliError::usage(
                "Changing the server requires an explicit key source (--key-stdin, --key-file, or --key-env).",
            ));
        }
    }
    if let Action::Profile(ProfileAction::Set(set)) = &action {
        let changed = set.server.is_some()
            || set.public_origin.is_some()
            || set.ca_bundle.is_some()
            || set.allow_http_loopback.is_some();
        if !changed {
            return Err(CliError::usage("profile set requires at least one change."));
        }
    }

    let limits = Limits {
        connect: global.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT),
        overall: global.timeout.unwrap_or(DEFAULT_OPERATION_TIMEOUT),
        read_retries: global.read_retries.unwrap_or(DEFAULT_READ_RETRIES),
    };

    Ok(Invocation {
        config: global.config.clone(),
        profile: global.profile.clone(),
        output,
        non_interactive: global.non_interactive,
        limits,
        key,
        action,
    })
}

fn key_count(global: &GlobalArgs) -> usize {
    usize::from(global.key_stdin)
        + usize::from(global.key_file.is_some())
        + usize::from(global.key_env.is_some())
}

fn key_source(global: &GlobalArgs) -> Result<Option<KeySource>, CliError> {
    if key_count(global) > 1 {
        return Err(CliError::usage(
            "Choose only one key source: --key-stdin, --key-file, or --key-env.",
        ));
    }
    if global.key_stdin {
        return Ok(Some(KeySource::Stdin));
    }
    if let Some(path) = &global.key_file {
        return Ok(Some(KeySource::File(path.clone())));
    }
    if let Some(name) = &global.key_env {
        crate::secret::validate_env_name(name)?;
        return Ok(Some(KeySource::Env(name.clone())));
    }
    Ok(None)
}

fn reject_shorten_args(shorten: &ShortenArgs) -> Result<(), CliError> {
    if shorten.title.is_some() || !shorten.tags.is_empty() || shorten.copy || shorten.no_copy {
        return Err(CliError::usage(
            "--title, --tag, --copy, and --no-copy apply only to creating a link.",
        ));
    }
    Ok(())
}

/// Combines `ferl create` options with any top-level shorthand options. Repeating a
/// scalar in both positions is a usage error.
fn merge_shorten_absent(target: &mut ShortenArgs, top: &ShortenArgs) -> Result<(), CliError> {
    if top.title.is_some() {
        if target.title.is_some() {
            return Err(CliError::usage("--title may be given only once."));
        }
        target.title = top.title.clone();
    }
    if !top.tags.is_empty() {
        target.tags.extend(top.tags.iter().cloned());
    }
    if top.copy {
        target.copy = true;
    }
    if top.no_copy {
        target.no_copy = true;
    }
    Ok(())
}

fn shorten_opts(destination: String, args: ShortenArgs) -> Result<ShortenOpts, CliError> {
    let copy = match (args.copy, args.no_copy) {
        (true, _) => CopyChoice::Always,
        (_, true) => CopyChoice::Never,
        _ => CopyChoice::Auto,
    };
    Ok(ShortenOpts {
        destination,
        title: args.title,
        tags: args.tags,
        copy,
    })
}

fn list_opts(args: ListArgs) -> Result<ListOpts, CliError> {
    let cursor = match (args.cursor, args.cursor_file) {
        (Some(value), None) => Some(CursorInput::Literal(value)),
        (None, Some(path)) => Some(CursorInput::File(path)),
        _ => None,
    };
    let columns = match args.columns {
        Some(text) => Some(parse_columns(&text)?),
        None => None,
    };
    Ok(ListOpts {
        query: args.query,
        limit: args.limit.unwrap_or(50),
        cursor,
        save_cursor: args.save_cursor,
        all: args.all,
        max_pages: args.max_pages.unwrap_or(100),
        columns,
    })
}

fn profile_action(args: ProfileArgs) -> Result<ProfileAction, CliError> {
    Ok(match args.action {
        None => ProfileAction::Wizard,
        Some(ProfileCommand::Add(add)) => ProfileAction::Add(AddOpts {
            name: add.name,
            server: add.server,
            public_origin: add.public_origin,
            ca_bundle: add.ca_bundle,
            allow_http_loopback: add.allow_http_loopback,
            make_default: add.make_default,
            validate: add.validate,
        }),
        Some(ProfileCommand::List) => ProfileAction::List,
        Some(ProfileCommand::Show(opt)) => ProfileAction::Show(opt.name),
        Some(ProfileCommand::Use(opt)) => ProfileAction::Use(opt.name),
        Some(ProfileCommand::Set(set)) => {
            if set.public_origin.is_some() && set.clear_public_origin {
                return Err(CliError::usage(
                    "Use either --public-origin or --clear-public-origin.",
                ));
            }
            let public_origin = if set.clear_public_origin {
                Some(None)
            } else {
                set.public_origin.map(Some)
            };
            let ca_bundle = if set.clear_ca_bundle {
                Some(None)
            } else {
                set.ca_bundle.map(Some)
            };
            let allow_http_loopback = if set.allow_http_loopback {
                Some(true)
            } else if set.disallow_http_loopback {
                Some(false)
            } else {
                None
            };
            ProfileAction::Set(SetOpts {
                name: set.name,
                server: set.server,
                public_origin,
                ca_bundle,
                allow_http_loopback,
            })
        }
        Some(ProfileCommand::SetKey(opts)) => ProfileAction::SetKey {
            name: opts.name,
            validate: opts.validate,
        },
        Some(ProfileCommand::Check(opt)) => ProfileAction::Check(opt.name),
        Some(ProfileCommand::Remove(opts)) => ProfileAction::Remove {
            name: opts.name,
            yes: opts.yes,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(args: &[&str]) -> Invocation {
        let argv: Vec<OsString> = std::iter::once("ferl")
            .chain(args.iter().copied())
            .map(OsString::from)
            .collect();
        match parse(argv, None).expect("parses") {
            ParseOutcome::Invocation(inv) => *inv,
            ParseOutcome::Text(_) => panic!("expected invocation"),
        }
    }

    fn parse_err(args: &[&str]) -> CliError {
        let argv: Vec<OsString> = std::iter::once("ferl")
            .chain(args.iter().copied())
            .map(OsString::from)
            .collect();
        parse(argv, None).expect_err("must be rejected")
    }

    #[test]
    fn shorthand_and_create_produce_the_same_action() {
        let short = parse_ok(&[
            "https://example.com/docs?edition=2#intro",
            "--title",
            "Team docs",
            "--tag",
            "Work",
        ]);
        let explicit = parse_ok(&[
            "create",
            "https://example.com/docs?edition=2#intro",
            "--title",
            "Team docs",
            "--tag",
            "Work",
        ]);
        match (short.action, explicit.action) {
            (Action::Shorten(a), Action::Shorten(b)) => {
                assert_eq!(a.destination, b.destination);
                assert_eq!(a.title, b.title);
                assert_eq!(a.tags, b.tags);
            }
            _ => panic!("expected shorten actions"),
        }
    }

    #[test]
    fn unknown_words_and_non_http_destinations_are_usage_errors() {
        assert_eq!(parse_err(&["frobnicate"]).exit_code(), 2);
        assert_eq!(parse_err(&["example.com"]).exit_code(), 2);
    }

    #[test]
    fn deferred_and_duplicate_options_fail_before_any_request() {
        assert_eq!(parse_err(&["list", "--scope", "all"]).exit_code(), 2);
        assert_eq!(
            parse_err(&["list", "--limit", "5", "--limit", "6"]).exit_code(),
            2
        );
        assert_eq!(
            parse_err(&["create", "https://a.example", "--custom-code", "x"]).exit_code(),
            2
        );
        assert_eq!(
            parse_err(&["create", "https://a.example", "https://b.example"]).exit_code(),
            2
        );
        assert_eq!(parse_err(&["list", "--max-pages", "5"]).exit_code(), 2);
        assert_eq!(parse_err(&["list", "--limit", "201"]).exit_code(), 2);
        assert_eq!(
            parse_err(&["list", "--cursor", "a", "--cursor-file", "b"]).exit_code(),
            2
        );
    }

    #[test]
    fn inapplicable_flags_are_usage_errors() {
        assert_eq!(parse_err(&["list", "--copy"]).exit_code(), 2);
        assert_eq!(parse_err(&["--title", "x", "list"]).exit_code(), 2);
        assert_eq!(
            parse_err(&["profile", "list", "--timeout", "5"]).exit_code(),
            2
        );
        assert_eq!(
            parse_err(&["delete", "abc", "--timeout", "5", "--connect-timeout", "9"]).exit_code(),
            2
        );
        assert_eq!(
            parse_err(&["profile", "list", "--profile", "work"]).exit_code(),
            2
        );
        assert_eq!(
            parse_err(&["list", "--key-env", "FERL_API_KEY"]).exit_code(),
            2
        );
        assert_eq!(
            parse_err(&["list", "--output", "json", "--columns", "title"]).exit_code(),
            2
        );
    }

    #[test]
    fn key_overrides_require_explicit_profile_and_one_source() {
        assert_eq!(parse_err(&["list", "--key-env", "WORK_KEY"]).exit_code(), 2);
        let ok = parse_ok(&["--profile", "work", "--key-env", "WORK_KEY", "list"]);
        assert!(ok.key.is_some());
        assert_eq!(
            parse_err(&["--profile", "w", "--key-stdin", "--key-env", "A", "list"]).exit_code(),
            2
        );
    }

    #[test]
    fn copy_flags_conflict_and_timeouts_are_bounded() {
        assert_eq!(
            parse_err(&["https://a.example", "--copy", "--no-copy"]).exit_code(),
            2
        );
        assert_eq!(parse_err(&["--timeout", "0", "list"]).exit_code(), 2);
        assert_eq!(parse_err(&["--timeout", "3601", "list"]).exit_code(), 2);
        assert_eq!(parse_err(&["--read-retries", "3", "list"]).exit_code(), 2);
        let inv = parse_ok(&["--timeout", "0.5", "list"]);
        assert_eq!(inv.limits.overall, Duration::from_millis(500));
    }

    #[test]
    fn delete_accepts_hyphen_codes_after_double_dash() {
        let inv = parse_ok(&["delete", "--", "-abc"]);
        match inv.action {
            Action::Delete(opts) => assert_eq!(opts.target, "-abc"),
            _ => panic!("expected delete"),
        }
    }

    #[test]
    fn profile_set_requires_a_change_and_key_for_server_changes() {
        assert_eq!(parse_err(&["profile", "set", "work"]).exit_code(), 2);
        assert_eq!(
            parse_err(&["profile", "set", "work", "--server", "https://a.example"]).exit_code(),
            2
        );
        assert!(matches!(
            parse_ok(&[
                "profile",
                "set",
                "work",
                "--server",
                "https://a.example",
                "--key-stdin"
            ])
            .action,
            Action::Profile(ProfileAction::Set(_))
        ));
    }

    #[test]
    fn json_detection_handles_flag_and_environment() {
        let args: Vec<OsString> = ["ferl", "--output", "json", "list"]
            .iter()
            .map(OsString::from)
            .collect();
        assert!(requests_json(&args, None));
        assert!(requests_json(&[], Some("json")));
        assert!(!requests_json(&[], None));
    }

    #[test]
    fn no_arguments_prints_help_without_a_command() {
        let argv = vec![OsString::from("ferl")];
        assert!(matches!(
            parse(argv, None).expect("help"),
            ParseOutcome::Text(_)
        ));
    }
}
