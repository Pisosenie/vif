//! Word from the system that its network changed: an address or a route came or went. Linux says
//! so on a netlink socket, macOS on a routing socket; both are only read, never written.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

/// How long changes keep coming after the first one before they count as one.
const SETTLE: Duration = Duration::from_millis(300);

pub struct Changes(OwnedFd);

impl Changes {
    pub fn watch() -> io::Result<Self> {
        open().map(Self)
    }

    /// Waits for the network to change and settle; Err when the socket can't be read any more.
    pub fn next(&self) -> io::Result<()> {
        let mut buf = [0u8; 8192];
        while !self.read(&mut buf, None)?.is_some_and(|n| matters(&buf[..n])) {}
        while self.read(&mut buf, Some(SETTLE))?.is_some() {}
        Ok(())
    }

    /// Reads one message, waiting at most `wait`: its length, or None when none came in time.
    fn read(&self, buf: &mut [u8], wait: Option<Duration>) -> io::Result<Option<usize>> {
        let fd = self.0.as_raw_fd();
        if let Some(wait) = wait {
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one pollfd that lives through the call.
            let n = unsafe { libc::poll(&mut poll, 1, wait.as_millis() as libc::c_int) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            if n == 0 {
                return Ok(None);
            }
        }
        loop {
            // SAFETY: reads into a buffer of the length given.
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n >= 0 {
                return Ok(Some(n as usize));
            }
            let err = io::Error::last_os_error();
            // A burst bigger than the socket's buffer is still a change, of whatever kind.
            if err.raw_os_error() == Some(libc::ENOBUFS) {
                return Ok(Some(0));
            }
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
    }
}

/// A socket fresh from `socket`, owned so it's closed however we leave.
fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a socket just made, which nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Whether a message is about addresses, routes or interfaces. Linux only sends those, by the
/// groups asked for; macOS sends every routing message, lookups and misses included.
#[cfg(target_os = "linux")]
fn matters(_: &[u8]) -> bool {
    true
}

#[cfg(target_os = "macos")]
fn matters(message: &[u8]) -> bool {
    // rtm_type is the fourth byte of every routing message; an empty read was a lost burst.
    message.get(3).is_none_or(|&kind| {
        [
            libc::RTM_ADD,
            libc::RTM_DELETE,
            libc::RTM_CHANGE,
            libc::RTM_NEWADDR,
            libc::RTM_DELADDR,
            libc::RTM_IFINFO,
        ]
        .contains(&i32::from(kind))
    })
}

#[cfg(target_os = "linux")]
fn open() -> io::Result<OwnedFd> {
    // SAFETY: plain socket creation; the result is checked.
    let fd = owned(unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    })?;
    // SAFETY: a C struct of plain numbers, for which all zeroes is a valid value.
    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    addr.nl_groups = (libc::RTMGRP_LINK
        | libc::RTMGRP_IPV4_IFADDR
        | libc::RTMGRP_IPV6_IFADDR
        | libc::RTMGRP_IPV4_ROUTE
        | libc::RTMGRP_IPV6_ROUTE) as u32;
    // SAFETY: binds our socket to a sockaddr_nl of the size given.
    let bound = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&raw const addr).cast(),
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if bound != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

#[cfg(target_os = "macos")]
fn open() -> io::Result<OwnedFd> {
    // SAFETY: plain socket creation; the result is checked.
    let fd = owned(unsafe { libc::socket(libc::PF_ROUTE, libc::SOCK_RAW, libc::AF_UNSPEC) })?;
    // SAFETY: sets close-on-exec on our own socket.
    unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    Ok(fd)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_system_lets_us_listen() {
        assert!(super::Changes::watch().is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn lookups_and_misses_are_no_change() {
        let message = |kind: i32| [0, 0, 5, kind as u8];
        assert!(super::matters(&message(libc::RTM_NEWADDR)));
        assert!(super::matters(&message(libc::RTM_DELETE)));
        assert!(!super::matters(&message(libc::RTM_MISS)));
        assert!(!super::matters(&message(libc::RTM_GET)));
        assert!(super::matters(&[]));
    }
}
