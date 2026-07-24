//! Locating and writing the `.ini` files (native mode).
//!
//! At runtime we prefer the `.ini.sample` shipped in `<install_dir>/etc` (it is
//! version-matched with the installed Arkime). If it is missing we fall back to
//! a copy compiled into the binary via `include_str!`, so the tool still works
//! standalone. Writes never overwrite an existing `.ini`, matching bash.

use std::borrow::Cow;
use std::io::ErrorKind;
use std::path::Path;

/// Which sample/config file we are dealing with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleKind {
    Config,
    Wise,
    Cont3xt,
    Parliament,
}

impl SampleKind {
    /// Base name without extension, e.g. `config` -> `config.ini` /
    /// `config.ini.sample`.
    pub fn base(self) -> &'static str {
        match self {
            SampleKind::Config => "config",
            SampleKind::Wise => "wise",
            SampleKind::Cont3xt => "cont3xt",
            SampleKind::Parliament => "parliament",
        }
    }

    fn embedded(self) -> &'static str {
        match self {
            SampleKind::Config => include_str!("../../templates/config.ini.sample"),
            SampleKind::Wise => include_str!("../../templates/wise.ini.sample"),
            SampleKind::Cont3xt => include_str!("../../templates/cont3xt.ini.sample"),
            SampleKind::Parliament => include_str!("../../templates/parliament.ini.sample"),
        }
    }
}

/// Load a sample, preferring the on-disk copy under `<etc_dir>` and falling back
/// to the embedded one.
pub fn load_sample(etc_dir: &Path, kind: SampleKind) -> Cow<'static, str> {
    let path = etc_dir.join(format!("{}.ini.sample", kind.base()));
    match std::fs::read_to_string(&path) {
        Ok(contents) => Cow::Owned(contents),
        Err(_) => Cow::Borrowed(kind.embedded()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Written,
    SkippedExists,
}

/// Write `contents` to `path` only if it does not already exist (bash:
/// `[ -f ... ] || write`). Uses `create_new` so the check-and-write is atomic.
pub fn write_if_absent(path: &Path, contents: &str) -> std::io::Result<WriteOutcome> {
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => {
            f.write_all(contents.as_bytes())?;
            Ok(WriteOutcome::Written)
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(WriteOutcome::SkippedExists),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_samples_are_present_and_nonempty() {
        for kind in [
            SampleKind::Config,
            SampleKind::Wise,
            SampleKind::Cont3xt,
            SampleKind::Parliament,
        ] {
            assert!(!kind.embedded().is_empty(), "{:?} embedded empty", kind);
        }
        // The real config sample carries the sentinels we substitute.
        let cfg = SampleKind::Config.embedded();
        assert!(cfg.contains("ARKIME_ELASTICSEARCH"));
        assert!(cfg.contains("ARKIME_PASSWORD"));
    }

    #[test]
    fn load_sample_prefers_disk_then_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        // Missing on disk -> embedded fallback.
        let fallback = load_sample(dir.path(), SampleKind::Config);
        assert!(fallback.contains("ARKIME_PASSWORD"));

        // Present on disk -> disk wins.
        std::fs::write(dir.path().join("config.ini.sample"), "on-disk=1\n").unwrap();
        let disk = load_sample(dir.path(), SampleKind::Config);
        assert_eq!(disk, "on-disk=1\n");
    }

    #[test]
    fn write_if_absent_does_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.ini");

        assert_eq!(
            write_if_absent(&path, "first").unwrap(),
            WriteOutcome::Written
        );
        assert_eq!(
            write_if_absent(&path, "second").unwrap(),
            WriteOutcome::SkippedExists
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
    }
}
