//! Install-dir / product-name resolution. Replaces the bash
//! `BUILD_ARKIME_INSTALL_DIR` sed placeholders.
//!
//! Defaults are baked at compile time via `option_env!` (set by the packaging
//! build if desired), and can be overridden at runtime with `--install-dir` /
//! `--name` for relocated installs and tests. The resolved value is threaded
//! explicitly through every templating/action call — no globals.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub name: String,
    pub install_dir: PathBuf,
}

impl BuildConfig {
    pub fn defaults() -> BuildConfig {
        let name = option_env!("BUILD_ARKIME_NAME").unwrap_or("arkime");
        let dir = option_env!("BUILD_ARKIME_INSTALL_DIR").unwrap_or("/opt/arkime");
        BuildConfig {
            name: name.to_string(),
            install_dir: PathBuf::from(dir),
        }
    }

    pub fn etc_dir(&self) -> PathBuf {
        self.install_dir.join("etc")
    }
}
