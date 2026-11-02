//! What runs in front in a terminal: its folder and its command line.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

/// The process group in front on the terminal, falling back to the shell.
fn front(pty: &File, shell: u32) -> i32 {
    let front = unsafe { libc::tcgetpgrp(pty.as_raw_fd()) };
    if front > 0 { front } else { shell as i32 }
}

/// The folder of whatever runs in front in the terminal, or of the shell itself.
pub fn cwd(pty: &File, shell: u32) -> Option<PathBuf> {
    dir(front(pty, shell)).or_else(|| dir(shell as i32))
}

/// The command line of whatever runs in front, like `ssh prod-db`.
pub fn command(pty: &File, shell: u32) -> Option<Vec<String>> {
    args(front(pty, shell))
}

#[cfg(target_os = "macos")]
fn dir(pid: i32) -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;

    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_vnodepathinfo>() as i32;
    let read = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, (&raw mut info).cast(), size) };
    if read != size {
        return None;
    }
    let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes()));
    path.is_dir().then_some(path)
}

#[cfg(not(target_os = "macos"))]
fn dir(pid: i32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(target_os = "macos")]
fn args(pid: i32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    let none = std::ptr::null_mut();
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, none, &mut size, none, 0) } != 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, none, 0) } != 0 {
        return None;
    }
    buf.truncate(size);
    // argc, the executable's path, padding, then the arguments, all NUL-separated.
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    let mut parts = buf[4..].split(|&b| b == 0).filter(|p| !p.is_empty());
    parts.next()?;
    Some(
        parts
            .take(argc)
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect(),
    )
}

#[cfg(not(target_os = "macos"))]
fn args(pid: i32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = raw
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    (!args.is_empty()).then_some(args)
}
