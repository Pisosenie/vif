//! What git knows of a folder, read from the repository's files without running git: the branch
//! it's on, so a tab can say it every second for nothing, and where its commits and files show
//! on the web.

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

/// HEAD of the repository `.git` stands for.
fn head_of(git: &Path) -> Option<PathBuf> {
    Some(git_dir(git)?.join("HEAD"))
}

/// The folder `.git` stands for: itself, or in a worktree or submodule the one its file names.
fn git_dir(git: &Path) -> Option<PathBuf> {
    if git.is_dir() {
        return Some(git.to_owned());
    }
    let pointer = fs::read_to_string(git).ok()?;
    let target = Path::new(pointer.trim().strip_prefix("gitdir: ")?);
    Some(match target.is_absolute() {
        true => target.to_owned(),
        false => git.parent()?.join(target),
    })
}

/// The repository's page on the web, from where `origin` points: GitHub, GitLab, Codeberg and
/// the like, over https or ssh.
fn web(git: &Path) -> Option<String> {
    let dir = git_dir(git)?;
    // A worktree keeps its config with the repository it came from.
    let common = fs::read_to_string(dir.join("commondir"))
        .map(|c| dir.join(c.trim()))
        .unwrap_or(dir);
    let config = fs::read_to_string(common.join("config")).ok()?;
    let mut in_origin = false;
    let url = config.lines().map(str::trim).find_map(|line| {
        if line.starts_with('[') {
            in_origin = line == "[remote \"origin\"]";
            return None;
        }
        let (key, value) = line.split_once('=')?;
        (in_origin && key.trim() == "url").then(|| value.trim().to_owned())
    })?;
    let url = url.trim_end_matches('/').trim_end_matches(".git");
    let (host, path) = if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        rest.split_once('/')?
    } else {
        // scp-like: git@host:owner/repo
        let rest = url.rsplit_once('@').map_or(url, |(_, r)| r);
        rest.split_once(':')?
    };
    let host = host.split(':').next()?;
    Some(format!("https://{host}/{}", path.trim_start_matches('/')))
}

/// Where a commit or a file of the repository around `dir` shows on the web, and a line in it.
pub fn page(dir: &Path, what: Page) -> Option<String> {
    let git = dir.ancestors().map(|d| d.join(".git")).find(|g| g.exists())?;
    let web = web(&git)?;
    // GitLab puts its pages under `/-/`; Gitea and Codeberg name the kind of ref.
    let (commit, tree) = if web.contains("gitlab") {
        ("-/commit", "-/blob")
    } else if web.contains("codeberg") || web.contains("gitea") {
        ("commit", "src/commit")
    } else {
        ("commit", "blob")
    };
    match what {
        Page::Commit(hash) => Some(format!("{web}/{commit}/{hash}")),
        Page::File(path, line) => {
            let root = git.parent()?;
            let rel = path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            let at = fs::read_to_string(head_of(&git)?).ok()?;
            let at = at.trim();
            // The commit HEAD is on, so the line is the one on screen whatever the branch.
            let rev = match at.strip_prefix("ref: ") {
                Some(name) => fs::read_to_string(git_dir(&git)?.join(name))
                    .ok()
                    .map(|r| r.trim().to_owned())
                    .unwrap_or_else(|| name.trim_start_matches("refs/heads/").to_owned()),
                None => at.to_owned(),
            };
            let anchor = line.map(|l| format!("#L{l}")).unwrap_or_default();
            Some(format!("{web}/{tree}/{rev}/{rel}{anchor}"))
        }
    }
}

pub enum Page<'a> {
    Commit(&'a str),
    File(&'a Path, Option<u32>),
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
    fn pages_on_the_web_for_commits_and_lines() {
        let root = repo("web");
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        fs::write(root.join(".git/refs/heads/main"), "abc1234def\n").unwrap();
        let file = root.join("src/deep/pane.rs");
        for (url, commit, line) in [
            (
                "git@github.com:Pisosenie/vif.git",
                "https://github.com/Pisosenie/vif/commit/8f3a2c1",
                "https://github.com/Pisosenie/vif/blob/abc1234def/src/deep/pane.rs#L12",
            ),
            (
                "https://gitlab.com/team/app",
                "https://gitlab.com/team/app/-/commit/8f3a2c1",
                "https://gitlab.com/team/app/-/blob/abc1234def/src/deep/pane.rs#L12",
            ),
            (
                "ssh://git@codeberg.org:22/me/tool.git",
                "https://codeberg.org/me/tool/commit/8f3a2c1",
                "https://codeberg.org/me/tool/src/commit/abc1234def/src/deep/pane.rs#L12",
            ),
        ] {
            fs::write(
                root.join(".git/config"),
                format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {url}\n"),
            )
            .unwrap();
            assert_eq!(
                page(&root.join("src"), Page::Commit("8f3a2c1")).as_deref(),
                Some(commit)
            );
            assert_eq!(page(&root, Page::File(&file, Some(12))).as_deref(), Some(line));
        }
        fs::write(root.join(".git/config"), "[core]\n").unwrap();
        assert_eq!(page(&root, Page::Commit("8f3a2c1")), None);
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
