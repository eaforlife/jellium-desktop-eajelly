//! Trusted GitHub-release installer handoff.

#[cfg(target_os = "windows")]
use sha2::{Digest, Sha256};
#[cfg(target_os = "windows")]
use std::io::{Read, Write};
#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "windows")]
static UPDATE_STARTED: AtomicBool = AtomicBool::new(false);

#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
const WINDOWS_ASSET_SUFFIX: &str = "-windows-arm64-setup.exe";
#[cfg(all(target_os = "windows", not(target_arch = "aarch64")))]
const WINDOWS_ASSET_SUFFIX: &str = "-windows-x64-setup.exe";

#[cfg(target_os = "windows")]
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/eaforlife/jellium-desktop-eajelly/releases/download/";
#[cfg(target_os = "windows")]
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;

#[cfg(target_os = "windows")]
fn trusted_installer_url(url: &str) -> bool {
    let Some(file_name) = url.rsplit('/').next() else {
        return false;
    };
    let file_name = file_name.replace("%2B", "+").replace("%2b", "+");
    url.starts_with(RELEASE_DOWNLOAD_PREFIX)
        && !file_name.is_empty()
        && file_name.starts_with("JelliumDesktop-")
        && file_name.ends_with(WINDOWS_ASSET_SUFFIX)
        && !file_name.contains('%')
        && file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
}

#[cfg(target_os = "windows")]
fn trusted_digest(digest: &str) -> bool {
    digest.len() == 71
        && digest.starts_with("sha256:")
        && digest[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Download the installer on a worker thread, launch it, then request a clean
/// application shutdown. Returns immediately so CEF's IPC thread never blocks.
#[cfg(target_os = "windows")]
pub(crate) fn install(url: &str, digest: &str, expected_size: u64) -> bool {
    if !trusted_installer_url(url)
        || !trusted_digest(digest)
        || expected_size == 0
        || expected_size > MAX_INSTALLER_BYTES
        || UPDATE_STARTED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return false;
    }

    let url = url.to_owned();
    let digest = digest.to_ascii_lowercase();
    let _ = std::thread::spawn(move || {
        if let Err(error) = download_and_launch(&url, &digest, expected_size) {
            jfn_logging::log(
                jfn_logging::CATEGORY_CEF,
                jfn_logging::LEVEL_ERROR,
                &format!("Update failed: {error}"),
            );
            UPDATE_STARTED.store(false, Ordering::Release);
        }
    });
    true
}

#[cfg(target_os = "windows")]
fn download_and_launch(
    url: &str,
    expected_digest: &str,
    expected_size: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    jfn_logging::log(
        jfn_logging::CATEGORY_CEF,
        jfn_logging::LEVEL_INFO,
        &format!("Downloading update from {url}"),
    );
    let mut response = ureq::get(url)
        .header(
            "User-Agent",
            concat!("JelliumDesktop/", env!("CARGO_PKG_VERSION")),
        )
        .call()?;

    let installer = std::env::temp_dir().join(format!(
        "JelliumDesktop-update-{}-setup.exe",
        std::process::id()
    ));
    let mut output = std::fs::File::create(&installer)?;
    let mut reader = response.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        if copied > expected_size || copied > MAX_INSTALLER_BYTES {
            return Err("downloaded installer exceeds its declared size".into());
        }
        hasher.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    if copied != expected_size {
        return Err("downloaded installer size does not match GitHub metadata".into());
    }
    let actual_digest = format!("sha256:{:x}", hasher.finalize());
    if actual_digest != expected_digest {
        return Err("downloaded installer SHA-256 does not match GitHub metadata".into());
    }
    output.flush()?;
    drop(output);

    // A minimal executable sanity check catches HTML/error payloads before
    // handing the file to Windows. Authenticity is provided by HTTPS and the
    // tightly restricted repository release URL above.
    let mut header = [0_u8; 2];
    std::fs::File::open(&installer)?.read_exact(&mut header)?;
    if header != *b"MZ" {
        return Err("downloaded update is not a Windows executable".into());
    }

    jfn_logging::log(
        jfn_logging::CATEGORY_CEF,
        jfn_logging::LEVEL_INFO,
        &format!("Launching installer: {}", installer.display()),
    );
    std::process::Command::new(&installer)
        .args(["/SP-", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS"])
        .spawn()?;
    jfn_playback::shutdown::jfn_shutdown_initiate();
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn install(_url: &str, _digest: &str, _expected_size: u64) -> bool {
    false
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_expected_repository_installer() {
        let suffix = WINDOWS_ASSET_SUFFIX;
        assert!(trusted_installer_url(&format!(
            "{RELEASE_DOWNLOAD_PREFIX}v3.0.3-eajelly/JelliumDesktop-3.0.3-eajelly%2Babc1234{suffix}"
        )));
        assert!(!trusted_installer_url(
            "https://example.com/JelliumDesktop-3.0.3-eajelly-windows-x64-setup.exe"
        ));
        assert!(!trusted_installer_url(&format!(
            "{RELEASE_DOWNLOAD_PREFIX}v3.0.3-eajelly/not-jellium{suffix}"
        )));
        assert!(trusted_digest(
            "sha256:0df798de3feefb1efc963dc809f83ad803a9392700fec57406a1a54042791ca6"
        ));
        assert!(!trusted_digest("sha256:not-a-digest"));
    }
}
