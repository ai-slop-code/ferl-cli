//! Account key input and handling. Keys are never accepted as argv values, never
//! echoed, and never included in diagnostics (business spec §4.2).

use std::fmt;
use std::io::{Read, stdin};
use std::path::PathBuf;

use crate::error::CliError;
use crate::storage::open_owner_only_file;

pub const MAX_KEY_INPUT_BYTES: usize = 65_536;

/// An account API key. `Debug` is redacted; the value is only exposed to the
/// transport when building the Authorization header.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Validates raw key input: one terminal LF or CRLF is stripped, then the value
    /// must be a single non-empty run of visible ASCII (safe in a Bearer header).
    pub fn from_input(mut raw: Vec<u8>) -> Result<Self, CliError> {
        if raw.len() > MAX_KEY_INPUT_BYTES + 2 {
            return Err(CliError::usage(
                "The key input exceeds the 65536-byte limit.",
            ));
        }
        if raw.ends_with(b"\r\n") {
            raw.truncate(raw.len() - 2);
        } else if raw.ends_with(b"\n") {
            raw.truncate(raw.len() - 1);
        }
        if raw.len() > MAX_KEY_INPUT_BYTES {
            return Err(CliError::usage(
                "The key input exceeds the 65536-byte limit.",
            ));
        }
        if raw.is_empty() {
            return Err(CliError::usage("The key input is empty."));
        }
        if raw.contains(&b'\n') || raw.contains(&b'\r') {
            return Err(CliError::usage("The key input must be a single line."));
        }
        if !raw.iter().all(|b| (0x21..=0x7e).contains(b)) {
            return Err(CliError::usage(
                "The key contains whitespace, control, or non-ASCII characters.",
            ));
        }
        let text = String::from_utf8(raw)
            .map_err(|_| CliError::usage("The key input is not valid ASCII."))?;
        Ok(ApiKey(text))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Explicit key source. Exactly one may be selected per invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    Stdin,
    File(PathBuf),
    Env(String),
}

impl KeySource {
    pub fn read(&self) -> Result<ApiKey, CliError> {
        match self {
            KeySource::Stdin => {
                let mut bytes = Vec::new();
                stdin()
                    .lock()
                    .take(MAX_KEY_INPUT_BYTES as u64 + 3)
                    .read_to_end(&mut bytes)
                    .map_err(|e| {
                        CliError::local_io(format!("Could not read the key from stdin: {e}."))
                    })?;
                ApiKey::from_input(bytes)
            }
            KeySource::File(path) => {
                let file = open_owner_only_file(path)?;
                let mut bytes = Vec::new();
                file.take(MAX_KEY_INPUT_BYTES as u64 + 3)
                    .read_to_end(&mut bytes)
                    .map_err(|e| {
                        CliError::local_io(format!("Could not read the key file: {e}."))
                    })?;
                ApiKey::from_input(bytes)
            }
            KeySource::Env(name) => {
                let value = std::env::var_os(name).ok_or_else(|| {
                    CliError::config(format!("The environment variable {name} is not set."))
                })?;
                let bytes = value.into_encoded_bytes();
                ApiKey::from_input(bytes)
            }
        }
    }
}

pub fn validate_env_name(name: &str) -> Result<(), CliError> {
    let mut chars = name.chars();
    let first_ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_');
    if first_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(CliError::usage(
            "An environment variable name must match [A-Za-z_][A-Za-z0-9_]*.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_newline_is_stripped_once_and_nothing_else_is_trimmed() {
        assert_eq!(
            ApiKey::from_input(b"abc123\n".to_vec())
                .expect("lf")
                .expose(),
            "abc123"
        );
        assert_eq!(
            ApiKey::from_input(b"abc123\r\n".to_vec())
                .expect("crlf")
                .expose(),
            "abc123"
        );
        assert!(ApiKey::from_input(b"abc123\n\n".to_vec()).is_err());
        assert!(ApiKey::from_input(b" abc123".to_vec()).is_err());
        assert!(ApiKey::from_input(b"abc 123".to_vec()).is_err());
        assert!(ApiKey::from_input(b"".to_vec()).is_err());
        assert!(ApiKey::from_input(b"\n".to_vec()).is_err());
        assert!(ApiKey::from_input("abcé".as_bytes().to_vec()).is_err());
    }

    #[test]
    fn key_input_size_is_bounded() {
        let at_limit = vec![b'a'; MAX_KEY_INPUT_BYTES];
        assert!(ApiKey::from_input(at_limit).is_ok());
        let over = vec![b'a'; MAX_KEY_INPUT_BYTES + 1];
        assert!(ApiKey::from_input(over).is_err());
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let key = ApiKey::from_input(b"ferl_ak_v1_sentinel".to_vec()).expect("key");
        let rendered = format!("{key:?}");
        assert!(!rendered.contains("sentinel"));
    }

    #[test]
    fn env_names_are_validated() {
        assert!(validate_env_name("FERL_API_KEY").is_ok());
        assert!(validate_env_name("1BAD").is_err());
        assert!(validate_env_name("BAD-NAME").is_err());
    }
}
