//! Secret files written restrictively at creation (KTD6): the mode is set by
//! `OpenOptions` before any bytes land, so no readable window exists, and
//! parent directories are created at `0o700`.

use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use crate::error::Failure;

/// Create `dir` and any missing parents at mode `0o700`.
///
/// # Errors
/// Returns [`Failure::Local`] when the directories cannot be created.
pub fn create_private_dirs(dir: &Path) -> Result<(), Failure> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| {
            Failure::local(
                format!("cannot create {}: {e}", dir.display()),
                "check filesystem permissions and retry",
            )
        })
}

/// Write `contents` to `path` as a new file with mode `0o600`, failing when
/// the file already exists: a second secret is always a fresh session, never
/// a silent overwrite.
///
/// # Errors
/// Returns [`Failure::Local`] when parents cannot be created, the file
/// exists, or the write fails.
pub fn write_secret(path: &Path, contents: &str) -> Result<(), Failure> {
    if let Some(parent) = path.parent() {
        create_private_dirs(parent)?;
    }
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true).mode(0o600);
    let mut file = opts.open(path).map_err(|e| {
        Failure::local(
            format!("cannot write secret {}: {e}", path.display()),
            "remove the stale file or start a new session",
        )
    })?;
    file.write_all(contents.as_bytes()).map_err(|e| {
        Failure::local(
            format!("cannot write secret {}: {e}", path.display()),
            "check filesystem permissions and retry",
        )
    })
}
