//! The git branch a folder is on, read from the repository's HEAD without running git, so a tab
//! can say it every second for nothing.

use std::fs;
use std::path::{Path, PathBuf};

/// The branch checked out where `dir` is, or the start of the commit when none is.
pub fn branch(dir: &Path) -> Option<String> {
    let git = dir.ancestors().map(|d| d.join(".git")).find(|g| g.exists())?;
    let head = fs::read_to_string(head_of(&git)?).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(name) => Some(name.strip_prefix("refs/heads/").unwrap_or(name).to_owned()),
        None => (head.len() >= 7 && head.bytes().all(|b| b.is_ascii_hexdigit())).then(|| head[..7].to_owned()),
    }
}

/// HEAD of the repository `.git` stands for: a folder, or in a worktree or submodule a file
/// that says where the folder is.
fn head_of(git: &Path) -> Option<PathBuf> {
    if git.is_dir() {
        return Some(git.join("HEAD"));
    }
    let pointer = fs::read_to_string(git).ok()?;
    let target = Path::new(pointer.trim().strip_prefix("gitdir: ")?);
    let target = match target.is_absolute() {
        true => target.to_owned(),
        false => git.parent()?.join(target),
    };
    Some(target.join("HEAD"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("vif-git-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("src/deep")).unwrap();
        root
    }

    #[test]
    fn a_branch_from_anywhere_in_the_tree() {
        let root = repo("branch");
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/udp-sessions\n").unwrap();
        assert_eq!(branch(&root.join("src/deep")).as_deref(), Some("udp-sessions"));
        fs::write(root.join(".git/HEAD"), "8f3a2c1d9e0b7a6f5e4d3c2b1a09f8e7d6c5b4a3\n").unwrap();
        assert_eq!(branch(&root).as_deref(), Some("8f3a2c1"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_worktree_points_to_its_own_head() {
        let root = repo("worktree");
        let real = root.join(".git/worktrees/fix");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("HEAD"), "ref: refs/heads/fix/login\n").unwrap();
        let tree = root.join("src/fix");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();
        assert_eq!(branch(&tree).as_deref(), Some("fix/login"));
        let _ = fs::remove_dir_all(&root);
    }
}
