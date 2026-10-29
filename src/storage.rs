//! Private local storage: owner-only permission checks, symlink refusal, advisory
//! locks, and crash-safe atomic replacement of individual files (business spec §4.1).
//! File contents are never included in diagnostics.

use std::fs::{self, DirBuilder, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use rustix::fs::{FlockOperation, OFlags, flock};

use crate::error::CliError;

/// Upper bound for any private file read into memory.
pub const MAX_PRIVATE_FILE_BYTES: u64 = 4 * 1024 * 1024;

fn effective_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn nofollow_flag() -> i32 {
    OFlags::NOFOLLOW.bits() as i32
}

fn unsafe_storage(path: &Path, reason: &str) -> CliError {
    CliError::new(
        crate::error::Kind::Configuration,
        "client.unsafe_storage",
        format!(
            "{} is not private to the current user ({reason}). Owner-only access (0700 directories, 0600 files, no ACL) is required.",
            path.display()
        ),
    )
}

/// Returns true when `ls -ld` reports an extended ACL (`+` in the mode column).
fn has_extended_acl(path: &Path) -> Result<bool, CliError> {
    let output = Command::new("/bin/ls")
        .arg("-ld")
        .arg(path)
        .output()
        .map_err(|_| CliError::config("Could not verify file access controls."))?;
    if !output.status.success() {
        return Err(CliError::config("Could not verify file access controls."));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mode = text.split_whitespace().next().unwrap_or("");
    Ok(mode.chars().nth(10) == Some('+'))
}

fn check_metadata(path: &Path, meta: &Metadata, want_dir: bool) -> Result<(), CliError> {
    if meta.file_type().is_symlink() {
        return Err(unsafe_storage(path, "symbolic links are refused"));
    }
    if want_dir && !meta.is_dir() {
        return Err(CliError::config(format!(
            "{} is not a directory.",
            path.display()
        )));
    }
    if !want_dir && !meta.is_file() {
        return Err(CliError::config(format!(
            "{} is not a regular file.",
            path.display()
        )));
    }
    if meta.uid() != effective_uid() {
        return Err(unsafe_storage(path, "it is not owned by the current user"));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(unsafe_storage(path, "group or other users have access"));
    }
    if has_extended_acl(path)? {
        return Err(unsafe_storage(path, "an access-control list is present"));
    }
    Ok(())
}

/// Verifies an existing directory, creating it with owner-only permissions when
/// `create` is set and it is missing.
pub fn ensure_private_dir(dir: &Path, create: bool) -> Result<(), CliError> {
    match fs::symlink_metadata(dir) {
        Ok(meta) => check_metadata(dir, &meta, true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|e| {
                    CliError::local_io(format!("Could not create {}: {e}.", dir.display()))
                })?;
            let meta = fs::symlink_metadata(dir).map_err(|e| CliError::local_io(e.to_string()))?;
            check_metadata(dir, &meta, true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(CliError::config(
            format!("Directory {} does not exist.", dir.display()),
        )),
        Err(error) => Err(CliError::config(format!(
            "Could not inspect {}: {error}.",
            dir.display()
        ))),
    }
}

/// Reads a private file, refusing symlinks and unsafe permissions. Returns `None`
/// when the file does not exist. The caller must hold the store lock.
pub fn read_private_file(path: &Path) -> Result<Option<String>, CliError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => check_metadata(path, &meta, false)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CliError::config(format!(
                "Could not inspect {}: {error}.",
                path.display()
            )));
        }
    }
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nofollow_flag())
        .open(path)
        .map_err(|e| CliError::config(format!("Could not open {}: {e}.", path.display())))?;
    let meta = file
        .metadata()
        .map_err(|e| CliError::local_io(e.to_string()))?;
    check_metadata(path, &meta, false)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_PRIVATE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| CliError::local_io(format!("Could not read {}: {e}.", path.display())))?;
    if bytes.len() as u64 > MAX_PRIVATE_FILE_BYTES {
        return Err(CliError::config(format!(
            "{} is too large to be a ferl file.",
            path.display()
        )));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| CliError::config(format!("{} is not valid UTF-8.", path.display())))
}

/// Replaces `path` atomically: stage a complete 0600 temporary file in the same
/// directory, fsync it, rename it over the target, and fsync the directory.
pub fn write_private_file_atomic(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let dir = path
        .parent()
        .ok_or_else(|| CliError::config("Storage path has no parent directory."))?;
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(unsafe_storage(path, "symbolic links are refused"));
    }
    let mut staged = tempfile::Builder::new()
        .prefix(".ferl-")
        .suffix(".tmp")
        .rand_bytes(12)
        .tempfile_in(dir)
        .map_err(|e| CliError::local_io(format!("Could not stage {}: {e}.", path.display())))?;
    staged
        .write_all(bytes)
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|e| CliError::local_io(format!("Could not write {}: {e}.", path.display())))?;
    staged.persist(path).map_err(|e| {
        CliError::local_io(format!(
            "Could not replace {}: {}.",
            path.display(),
            e.error
        ))
    })?;
    sync_dir(dir)
}

fn sync_dir(dir: &Path) -> Result<(), CliError> {
    File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| CliError::local_io(format!("Could not sync {}: {e}.", dir.display())))
}

/// Holds an advisory lock on the store lock file for the lifetime of the value.
#[derive(Debug)]
pub struct StoreLock {
    _file: File,
}

pub fn lock_store(lock_path: &Path, exclusive: bool) -> Result<StoreLock, CliError> {
    if let Ok(meta) = fs::symlink_metadata(lock_path) {
        check_metadata(lock_path, &meta, false)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(nofollow_flag())
        .open(lock_path)
        .map_err(|e| CliError::config(format!("Could not open {}: {e}.", lock_path.display())))?;
    let meta = file
        .metadata()
        .map_err(|e| CliError::local_io(e.to_string()))?;
    check_metadata(lock_path, &meta, false)?;
    let op = if exclusive {
        FlockOperation::LockExclusive
    } else {
        FlockOperation::LockShared
    };
    flock(&file, op)
        .map_err(|e| CliError::local_io(format!("Could not lock {}: {e}.", lock_path.display())))?;
    Ok(StoreLock { _file: file })
}

/// Makes an absolute path from a user-supplied relative one without shell expansion.
pub fn absolutize(path: &Path) -> Result<PathBuf, CliError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|e| CliError::config(format!("Could not resolve the current directory: {e}.")))
}

/// Checks a user-owned secret input file (`--key-file`) without reading it first.
pub fn open_owner_only_file(path: &Path) -> Result<File, CliError> {
    let meta = fs::symlink_metadata(path).map_err(|e| {
        CliError::config(format!(
            "Could not inspect key file {}: {e}.",
            path.display()
        ))
    })?;
    check_metadata(path, &meta, false)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(nofollow_flag())
        .open(path)
        .map_err(|e| {
            CliError::config(format!("Could not open key file {}: {e}.", path.display()))
        })?;
    let opened = file
        .metadata()
        .map_err(|e| CliError::local_io(e.to_string()))?;
    check_metadata(path, &opened, false)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn private_tempdir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).expect("chmod");
        dir
    }

    #[test]
    fn atomic_write_creates_owner_only_file_and_replaces_contents() {
        let dir = private_tempdir();
        let path = dir.path().join("config.toml");
        write_private_file_atomic(&path, b"first").expect("first write");
        write_private_file_atomic(&path, b"second").expect("second write");
        assert_eq!(
            read_private_file(&path).expect("read").as_deref(),
            Some("second")
        );
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn group_readable_file_is_rejected_without_repair() {
        let dir = private_tempdir();
        let path = dir.path().join("config.toml");
        fs::write(&path, b"x").expect("write");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("chmod");
        let err = read_private_file(&path).expect_err("unsafe mode must fail");
        assert_eq!(err.exit_code(), 3);
        let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o640,
            "unsafe permissions must not be silently repaired"
        );
    }

    #[test]
    fn symlinks_are_refused_for_reads_and_replacements() {
        let dir = private_tempdir();
        let target = dir.path().join("target");
        fs::write(&target, b"secret-looking").expect("write");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("chmod");
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert_eq!(
            read_private_file(&link).expect_err("symlink").exit_code(),
            3
        );
        assert_eq!(
            write_private_file_atomic(&link, b"new")
                .expect_err("symlink")
                .exit_code(),
            3
        );
        assert_eq!(fs::read(&target).expect("read"), b"secret-looking");
    }

    #[test]
    fn missing_file_is_absent_not_an_error() {
        let dir = private_tempdir();
        assert!(
            read_private_file(&dir.path().join("none.toml"))
                .expect("read")
                .is_none()
        );
    }

    #[test]
    fn explicit_directory_must_be_private() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).expect("chmod");
        assert_eq!(
            ensure_private_dir(dir.path(), false)
                .expect_err("public")
                .exit_code(),
            3
        );
    }

    #[test]
    fn owned_private_directory_is_accepted_and_missing_directory_is_created_private() {
        let dir = tempfile::tempdir().expect("tempdir");
        let child = dir.path().join("created");
        ensure_private_dir(&child, true).expect("create");
        let mode = fs::metadata(&child).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn concurrent_writers_serialize_on_the_lock_file() {
        let dir = private_tempdir();
        let lock_path = dir.path().join("config.toml.lock");
        let first = lock_store(&lock_path, true).expect("first lock");
        let (tx, rx) = std::sync::mpsc::channel();
        let path = lock_path.clone();
        let handle = std::thread::spawn(move || {
            let second = lock_store(&path, true);
            tx.send(second.is_ok()).expect("send");
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "second writer must wait while the first holds the lock"
        );
        drop(first);
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(5))
                .expect("second acquires")
        );
        handle.join().expect("join");
    }
}
