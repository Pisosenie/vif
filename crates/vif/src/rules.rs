//! Rules: a pane ssh'd into a matching host, or sitting in a matching folder, takes the rule's
//! look, so production never looks like your laptop.

use std::path::{Path, PathBuf};

use vif_config::Rule;

/// Where a pane is right now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub cwd: Option<PathBuf>,
    pub host: Option<String>,
    /// What runs in front when it isn't the shell, as typed.
    pub running: Option<String>,
}

impl Place {
    pub fn read(cwd: Option<PathBuf>, front: Option<Vec<String>>) -> Self {
        Self {
            cwd,
            host: front.as_deref().and_then(ssh_host),
            running: front.filter(|argv| !crate::process::is_shell(argv)).map(|argv| {
                let name = argv[0].rsplit('/').next().unwrap_or(&argv[0]).to_owned();
                std::iter::once(name)
                    .chain(argv[1..].iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" ")
            }),
        }
    }
}

/// The first rule that fits this place.
pub fn pick(rules: &[Rule], place: &Place) -> Option<usize> {
    rules.iter().position(|rule| {
        let host = rule
            .host
            .as_deref()
            .is_none_or(|pattern| place.host.as_deref().is_some_and(|host| glob(pattern, host)));
        let dir = rule
            .dir
            .as_deref()
            .is_none_or(|pattern| place.cwd.as_deref().is_some_and(|cwd| inside(pattern, cwd)));
        host && dir
    })
}

/// Whether `cwd` or a folder above it fits the pattern.
fn inside(pattern: &str, cwd: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let pattern = match (pattern.strip_prefix("~"), &home) {
        (Some(rest), Some(home)) => format!("{}{rest}", home.display()),
        _ => pattern.to_owned(),
    };
    let pattern = pattern.trim_end_matches('/');
    cwd.ancestors().any(|dir| glob(pattern, &dir.display().to_string()))
}

/// The host an ssh command line connects to, without the user and port.
pub fn ssh_host(argv: &[String]) -> Option<String> {
    let program = argv.first()?.rsplit('/').next()?;
    if program != "ssh" {
        return None;
    }
    let with_value = "BbcDEeFIiJLlmOoPpQRSWw";
    let mut args = argv[1..].iter();
    while let Some(arg) = args.next() {
        if let Some(flags) = arg.strip_prefix('-') {
            if flags.is_empty() || flags == "-" {
                return args.next().map(|a| host_of(a));
            }
            // -p 22 takes the next word; -p22 doesn't.
            if let Some(last) = flags.chars().position(|c| with_value.contains(c))
                && last == flags.len() - 1
            {
                args.next();
            }
            continue;
        }
        return Some(host_of(arg));
    }
    None
}

fn host_of(target: &str) -> String {
    let target = target.strip_prefix("ssh://").unwrap_or(target);
    let target = target.rsplit('@').next().unwrap_or(target);
    target.split(':').next().unwrap_or(target).to_owned()
}

/// `*` for any run of characters, `?` for one; the rest must match exactly, ignoring case.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (
        pattern.to_lowercase().chars().collect(),
        text.to_lowercase().chars().collect(),
    );
    let (mut pi, mut ti) = (0, 0);
    let mut back: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some('*') => {
                back = Some((pi, ti));
                pi += 1;
            }
            Some(&c) if c == '?' || c == t[ti] => {
                pi += 1;
                ti += 1;
            }
            _ => match back {
                Some((bp, bt)) => {
                    pi = bp + 1;
                    ti = bt + 1;
                    back = Some((bp, bt + 1));
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn finds_the_host_ssh_goes_to() {
        assert_eq!(ssh_host(&argv("ssh prod-db")).as_deref(), Some("prod-db"));
        assert_eq!(
            ssh_host(&argv("/usr/bin/ssh -p 2222 -i key danil@prod-db.local")).as_deref(),
            Some("prod-db.local")
        );
        assert_eq!(
            ssh_host(&argv("ssh -tA -o ForwardAgent=yes ssh://root@10.0.0.5:22 uptime")).as_deref(),
            Some("10.0.0.5")
        );
        assert_eq!(ssh_host(&argv("ssh -p22 box")).as_deref(), Some("box"));
        assert_eq!(ssh_host(&argv("vim notes")), None);
    }

    #[test]
    fn globs_and_folders() {
        assert!(glob("prod-*", "prod-db"));
        assert!(glob("*.prod.*", "api.prod.example.com"));
        assert!(!glob("prod-*", "staging-db"));
        assert!(glob("db?", "DB1"));
        assert!(inside("/srv/infra*", Path::new("/srv/infra-live/terraform")));
        assert!(!inside("/srv/infra*", Path::new("/srv/other")));
    }

    #[test]
    fn the_first_fitting_rule_wins() {
        let rule = |host: Option<&str>, dir: Option<&str>| Rule {
            host: host.map(str::to_owned),
            dir: dir.map(str::to_owned),
            theme: None,
            accent: None,
            background: None,
            label: None,
        };
        let rules = [rule(Some("prod-*"), None), rule(None, Some("/srv"))];
        let place = Place {
            cwd: Some(PathBuf::from("/srv/app")),
            host: Some("prod-db".to_owned()),
            ..Place::default()
        };
        assert_eq!(pick(&rules, &place), Some(0));
        let local = Place {
            cwd: Some(PathBuf::from("/srv/app")),
            ..Place::default()
        };
        assert_eq!(pick(&rules, &local), Some(1));
        assert_eq!(pick(&rules, &Place::default()), None);
    }
}
