//! Folders only this user can get into, for sockets and what the sessions keep. A folder of
//! ours that someone else made first, or opened up, could let them swap a socket and read
//! what is typed, so vif checks before it uses one.

use std::fs::{self, DirBuilder, Permissions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// XDG_RUNTIME_DIR, which the login made for this user alone, or a folder of ours in the
/// temporary folder, which everyone shares; true for the second.
fn base() -> (PathBuf, bool) {
    match std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| closed(dir))
    {
        Some(dir) => (dir, false),
        None => (std::env::temp_dir().join(format!("vif-{}", uid())), true),
    }
}

/// Whether `dir` is this user's own real folder that nobody else can write into; an
/// XDG_RUNTIME_DIR that isn't is no safer than the temporary folder, so it isn't trusted.
/// Others may read it: what vif keeps goes in folders of its own inside, made 0700.
fn closed(dir: &Path) -> bool {
    fs::symlink_metadata(dir).is_ok_and(|m| own(&m) && m.mode() & 0o022 == 0)
}

/// A real folder, not a link to one, that this user owns.
fn own(meta: &fs::Metadata) -> bool {
    meta.is_dir() && meta.uid() == uid()
}

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and can't fail.
    unsafe { libc::getuid() }
}

/// Where vif's sockets go, without making anything.
pub fn runtime() -> PathBuf {
    base().0
}

/// [`runtime`], made and checked when it's ours to make.
pub fn runtime_made() -> io::Result<PathBuf> {
    let (dir, ours) = base();
    if ours {
        make(&dir)?;
    }
    Ok(dir)
}

/// `name` inside [`runtime`], made and checked.
pub fn made(name: &str) -> io::Result<PathBuf> {
    let dir = runtime_made()?.join(name);
    make(&dir)?;
    Ok(dir)
}

/// Makes `dir` closed to others, or checks the one there is this user's own real folder and
/// closes it.
fn make(dir: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(0o700).create(dir) {
        Err(err) if err.kind() != io::ErrorKind::AlreadyExists => return Err(err),
        _ => {}
    }
    let meta = fs::symlink_metadata(dir)?;
    if !own(&meta) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not this user's own folder, so vif won't use it", dir.display()),
        ));
    }
    if meta.mode() & 0o077 != 0 {
        fs::set_permissions(dir, Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_made_closed_and_links_refused() {
        let root = std::env::temp_dir().join(format!("vif-private-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let open = root.join("open");
        fs::create_dir(&open).unwrap();
        fs::set_permissions(&open, Permissions::from_mode(0o777)).unwrap();
        make(&open).unwrap();
        assert_eq!(fs::metadata(&open).unwrap().mode() & 0o777, 0o700);
        let fresh = root.join("fresh");
        make(&fresh).unwrap();
        assert_eq!(fs::metadata(&fresh).unwrap().mode() & 0o777, 0o700);
        let link = root.join("link");
        std::os::unix::fs::symlink(&open, &link).unwrap();
        assert_eq!(make(&link).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_runtime_folder_others_can_write_into_is_not_used() {
        let root = std::env::temp_dir().join(format!("vif-runtime-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let inner = root.join("inner");
        fs::create_dir(&inner).unwrap();
        fs::set_permissions(&inner, Permissions::from_mode(0o755)).unwrap();
        assert!(closed(&inner));
        fs::set_permissions(&inner, Permissions::from_mode(0o777)).unwrap();
        assert!(!closed(&inner), "others can write into it");
        let link = root.join("link");
        std::os::unix::fs::symlink(&inner, &link).unwrap();
        assert!(!closed(&link), "a link");
        assert!(!closed(Path::new("")));
        fs::remove_dir_all(&root).unwrap();
    }
}
