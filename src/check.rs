// https://github.com/Dentosal/wsl-rs
// but with explicit v2 check - we don't care about WSL1

#[allow(dead_code)]
const OS_RELEASE: &str = "/proc/sys/kernel/osrelease";

pub fn is_not_wsl2() -> bool {
    !is_wsl2()
}

#[cfg(target_os = "linux")]
pub fn is_wsl2() -> bool {
    if let Ok(b) = std::fs::read(OS_RELEASE) {
        if let Ok(s) = std::str::from_utf8(&b) {
            let a = s.to_ascii_lowercase();
            return a.contains("wsl2");
        }
    }
    false
}

/// Test if the program is running under WSL
#[cfg(not(target_os = "linux"))]
pub fn is_wsl2() -> bool {
    false
}
