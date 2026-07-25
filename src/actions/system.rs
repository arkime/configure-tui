//! Side-effect boundary. All file writes / process spawns go through `SystemOps`
//! so the native and docker flows can be unit-tested against a recording fake
//! that captures intended operations without touching the host.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Run {
        program: String,
        args: Vec<String>,
    },
    WriteNew {
        path: PathBuf,
        existed: bool,
    },
    WriteFile {
        path: PathBuf,
        mode: u32,
    },
    Mkdir {
        path: PathBuf,
        mode: u32,
        owner: String,
    },
    Backup {
        path: PathBuf,
    },
}

pub trait SystemOps {
    /// Run a command, returning an error on non-zero exit.
    fn run(&self, program: &str, args: &[&str]) -> Result<()>;
    /// Write only if the file is absent (bash `[ -f ] || write`). Returns whether
    /// the file already existed.
    fn write_new(&self, path: &Path, contents: &str) -> Result<bool>;
    /// Write/overwrite a file with an explicit unix mode.
    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> Result<()>;
    /// `mkdir -m <mode> -p <path>` then `chown <owner> <path>` if it does not
    /// already exist.
    fn mkdir_mode_chown(&self, path: &Path, mode: u32, owner: &str) -> Result<()>;
    /// Copy an existing file to a timestamped `.bak` before it is overwritten.
    /// Returns the backup path, or None if there was nothing to back up.
    fn backup(&self, path: &Path) -> Result<Option<PathBuf>>;
}

/// Production implementation backed by std + `nix`.
pub struct RealOps;

impl SystemOps for RealOps {
    fn run(&self, program: &str, args: &[&str]) -> Result<()> {
        let status = std::process::Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("failed to spawn {program}"))?;
        if !status.success() {
            anyhow::bail!("{program} exited with {status}");
        }
        Ok(())
    }

    fn write_new(&self, path: &Path, contents: &str) -> Result<bool> {
        use crate::config::templates::{write_if_absent, WriteOutcome};
        match write_if_absent(path, contents)? {
            WriteOutcome::Written => Ok(false),
            WriteOutcome::SkippedExists => Ok(true),
        }
    }

    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> Result<()> {
        std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
        set_mode(path, mode)
    }

    fn mkdir_mode_chown(&self, path: &Path, mode: u32, owner: &str) -> Result<()> {
        if path.exists() {
            return Ok(());
        }
        std::fs::create_dir_all(path).with_context(|| format!("mkdir {}", path.display()))?;
        set_mode(path, mode)?;
        chown(path, owner)
    }

    fn backup(&self, path: &Path) -> Result<Option<PathBuf>> {
        if !path.exists() {
            return Ok(None);
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".{secs}.bak"));
        let bak = PathBuf::from(name);
        std::fs::copy(path, &bak)
            .with_context(|| format!("backing up {} -> {}", path.display(), bak.display()))?;
        Ok(Some(bak))
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("chmod {:o} {}", mode, path.display()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn chown(path: &Path, owner: &str) -> Result<()> {
    let user = nix::unistd::User::from_name(owner)
        .with_context(|| format!("looking up user {owner}"))?
        .with_context(|| format!("user {owner} not found"))?;
    std::os::unix::fs::chown(path, Some(user.uid.as_raw()), None)
        .with_context(|| format!("chown {owner} {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn chown(_path: &Path, _owner: &str) -> Result<()> {
    Ok(())
}

/// Test double: records every op, never touches the filesystem. `existing` marks
/// paths that should report as already-present for `write_new`.
#[cfg(test)]
pub struct RecordingOps {
    pub ops: std::cell::RefCell<Vec<Op>>,
    pub existing: std::collections::HashSet<PathBuf>,
}

#[cfg(test)]
impl Default for RecordingOps {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl RecordingOps {
    pub fn new() -> Self {
        RecordingOps {
            ops: std::cell::RefCell::new(Vec::new()),
            existing: std::collections::HashSet::new(),
        }
    }

    pub fn ops(&self) -> Vec<Op> {
        self.ops.borrow().clone()
    }
}

#[cfg(test)]
impl SystemOps for RecordingOps {
    fn run(&self, program: &str, args: &[&str]) -> Result<()> {
        self.ops.borrow_mut().push(Op::Run {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        });
        Ok(())
    }

    fn write_new(&self, path: &Path, _contents: &str) -> Result<bool> {
        let existed = self.existing.contains(path);
        self.ops.borrow_mut().push(Op::WriteNew {
            path: path.to_path_buf(),
            existed,
        });
        Ok(existed)
    }

    fn write_file(&self, path: &Path, _contents: &str, mode: u32) -> Result<()> {
        self.ops.borrow_mut().push(Op::WriteFile {
            path: path.to_path_buf(),
            mode,
        });
        Ok(())
    }

    fn mkdir_mode_chown(&self, path: &Path, mode: u32, owner: &str) -> Result<()> {
        self.ops.borrow_mut().push(Op::Mkdir {
            path: path.to_path_buf(),
            mode,
            owner: owner.to_string(),
        });
        Ok(())
    }

    fn backup(&self, path: &Path) -> Result<Option<PathBuf>> {
        let existed = self.existing.contains(path);
        self.ops.borrow_mut().push(Op::Backup {
            path: path.to_path_buf(),
        });
        Ok(existed.then(|| path.with_extension("bak")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_copies_existing_and_skips_missing() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.ini");
        // Nothing to back up yet.
        assert!(RealOps.backup(&f).unwrap().is_none());

        std::fs::write(&f, "original").unwrap();
        let bak = RealOps.backup(&f).unwrap().expect("a backup path");
        assert!(bak.exists());
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "original");
        assert!(bak.to_string_lossy().ends_with(".bak"));
        // The original is untouched.
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "original");
    }
}
