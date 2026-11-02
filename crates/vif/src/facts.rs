//! Who, where and on what: the lines the greeting shows beside its picture, read straight from
//! the system without running anything.

use std::ffi::CStr;
use std::fs;

/// Labelled lines, in the order they show.
pub fn gather() -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut push = |label, value: Option<String>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            out.push((label, value));
        }
    };
    push("os", os());
    push("kernel", kernel());
    push("uptime", uptime().map(span));
    push(
        "shell",
        std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(str::to_owned)),
    );
    push("desktop", desktop());
    push("terminal", Some(format!("vif {}", env!("CARGO_PKG_VERSION"))));
    push("cpu", cpu());
    push(
        "memory",
        memory().map(|bytes| format!("{:.0} GiB", bytes as f64 / f64::from(1u32 << 30))),
    );
    out
}

/// `user@host`.
pub fn who() -> String {
    let user = std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| {
        let entry = unsafe { libc::getpwuid(libc::getuid()) };
        if entry.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr((*entry).pw_name) }
            .to_string_lossy()
            .into_owned()
    });
    let mut buf = [0u8; 256];
    let host = if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0 {
        CStr::from_bytes_until_nul(&buf)
            .map(|h| h.to_string_lossy().trim_end_matches(".local").to_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!("{user}@{host}")
}

fn uname() -> Option<libc::utsname> {
    let mut name: libc::utsname = unsafe { std::mem::zeroed() };
    (unsafe { libc::uname(&mut name) } == 0).then_some(name)
}

fn field(raw: &[libc::c_char]) -> String {
    unsafe { CStr::from_ptr(raw.as_ptr()) }.to_string_lossy().into_owned()
}

fn kernel() -> Option<String> {
    uname().map(|u| format!("{} {}", field(&u.sysname), field(&u.release)))
}

fn span(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes} min"),
        (0, _) => format!("{hours} h {minutes} min"),
        _ => format!("{days} d {hours} h"),
    }
}

#[cfg(target_os = "linux")]
fn os() -> Option<String> {
    let text = fs::read_to_string("/etc/os-release").ok()?;
    let value = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim_matches('"').to_owned())
    };
    value("PRETTY_NAME").or_else(|| value("NAME"))
}

#[cfg(target_os = "macos")]
fn os() -> Option<String> {
    let plist = fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    let after = plist.split("<key>ProductVersion</key>").nth(1)?;
    let version = after.split("<string>").nth(1)?.split("</string>").next()?;
    Some(format!("macOS {version}"))
}

#[cfg(target_os = "linux")]
fn uptime() -> Option<u64> {
    let text = fs::read_to_string("/proc/uptime").ok()?;
    text.split('.').next()?.parse().ok()
}

#[cfg(target_os = "macos")]
fn uptime() -> Option<u64> {
    let boot: libc::timeval = sysctl("kern.boottime")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    now.as_secs().checked_sub(boot.tv_sec as u64)
}

#[cfg(target_os = "linux")]
fn desktop() -> Option<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .or_else(|| std::env::var("NIRI_SOCKET").is_ok().then(|| "niri".to_owned()))
}

#[cfg(target_os = "macos")]
fn desktop() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn cpu() -> Option<String> {
    let text = fs::read_to_string("/proc/cpuinfo").ok()?;
    let model = text.lines().find_map(|l| {
        l.strip_prefix("model name")?
            .split_once(':')
            .map(|(_, v)| v.trim().to_owned())
    });
    let count = text.lines().filter(|l| l.starts_with("processor")).count();
    let model = model.or_else(|| uname().map(|u| field(&u.machine)))?;
    Some(if count > 1 { format!("{model} ×{count}") } else { model })
}

#[cfg(target_os = "macos")]
fn cpu() -> Option<String> {
    let mut buf = [0u8; 128];
    let mut len = buf.len();
    let ok = unsafe {
        libc::sysctlbyname(
            c"machdep.cpu.brand_string".as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    ok.then(|| {
        CStr::from_bytes_until_nul(&buf)
            .ok()
            .map(|c| c.to_string_lossy().into_owned())
    })
    .flatten()
}

#[cfg(target_os = "linux")]
fn memory() -> Option<u64> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    let kib: u64 = text
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kib * 1024)
}

#[cfg(target_os = "macos")]
fn memory() -> Option<u64> {
    sysctl("hw.memsize")
}

#[cfg(target_os = "macos")]
fn sysctl<T: Copy>(name: &str) -> Option<T> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let mut len = size_of::<T>();
    let ok = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    (ok && len == size_of::<T>()).then(|| unsafe { value.assume_init() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_read_short() {
        assert_eq!(span(59), "0 min");
        assert_eq!(span(3 * 3600 + 12 * 60), "3 h 12 min");
        assert_eq!(span(2 * 86400 + 5 * 3600), "2 d 5 h");
    }

    #[test]
    fn knows_this_machine() {
        let facts = gather();
        assert!(facts.iter().any(|(label, _)| *label == "kernel"));
        assert!(who().contains('@'));
    }
}
