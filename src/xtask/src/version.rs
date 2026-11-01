use crate::paths;
use anyhow::{Context, Result, anyhow};

pub struct Version {
    /// Adds a Git suffix to non-tag prereleases, but keeps release-tag builds
    /// identical to the tag's version string.
    pub full: String,
    /// Numeric `major.minor.patch` form for platform metadata that rejects
    /// SemVer pre-release/build suffixes (notably macOS bundle versions).
    #[cfg(target_os = "macos")]
    pub numeric: String,
}

/// Short HEAD hash and dirty flag. `(None, false)` when there is no repo.
pub fn git_info() -> (Option<String>, bool) {
    let Ok(repo) = gix::discover(paths::repo_root()) else {
        return (None, false);
    };
    let hash = repo
        .head_id()
        .ok()
        .map(|id| id.to_hex_with_len(7).to_string());
    let dirty = repo.is_dirty().unwrap_or(false);
    (hash, dirty)
}

pub fn read() -> Result<Version> {
    let raw = env!("CARGO_PKG_VERSION").to_string();
    #[cfg(target_os = "macos")]
    let numeric = raw.split('-').next().unwrap_or(&raw).to_string();
    let is_tag_build = matches!(std::env::var("GITHUB_REF_TYPE").as_deref(), Ok("tag"))
        && std::env::var("GITHUB_REF_NAME").is_ok_and(|name| name == format!("v{raw}"));
    let full = if raw.contains('-') && !is_tag_build {
        match git_info() {
            (Some(hash), dirty) => {
                let suffix = if dirty { "-dirty" } else { "" };
                format!("{raw}+{hash}{suffix}")
            }
            _ => raw,
        }
    } else {
        raw
    };
    Ok(Version {
        full,
        #[cfg(target_os = "macos")]
        numeric,
    })
}

pub fn cef_package_version() -> Result<String> {
    let lock = paths::repo_root().join("src").join("Cargo.lock");
    let lockfile =
        cargo_lock::Lockfile::load(&lock).with_context(|| format!("parse {}", lock.display()))?;
    lockfile
        .packages
        .iter()
        .find(|pkg| pkg.name.as_str() == "cef")
        .map(|pkg| pkg.version.to_string())
        .ok_or_else(|| anyhow!("`cef` package not found in {}", lock.display()))
}
