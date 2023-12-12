// https://github.com/Dentosal/wsl-rs
// but with explicit v2 check - we don't care about WSL1

use std::sync::OnceLock;

pub(crate) fn wsl2_or_exit() -> crate::NullResult {
    if !_is_wsl2() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "This command can only be used in a WSL2 environment",
        )
        .into());
    }
    Ok(())
}

pub(crate) fn is_wsl2() -> bool {
    static IS_WSL2: OnceLock<bool> = OnceLock::new();
    *IS_WSL2.get_or_init(_is_wsl2)
}

#[cfg(target_os = "linux")]
fn _is_wsl2() -> bool {
    const OS_RELEASE: &str = "/proc/sys/kernel/osrelease";
    const MARKER: &str = "wsl2";

    std::fs::read_to_string(OS_RELEASE)
        .map(|os_release| os_release.to_ascii_lowercase().contains(MARKER))
        .unwrap_or_default()
}

/// Test if the program is running under WSL
#[cfg(not(target_os = "linux"))]
fn _is_wsl2() -> bool {
    false
}
