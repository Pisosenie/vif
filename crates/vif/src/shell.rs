//! Shell integration: the shell marks its prompts, commands and output (OSC 133) without the
//! user touching their rc files. zsh comes in through ZDOTDIR, bash through --rcfile; fish 4
//! marks prompts on its own.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use vif_config::Config;

const FILES: [(&str, &str); 3] = [
    ("zsh/.zshenv", include_str!("../../../assets/shell/zshenv")),
    ("zsh/vif.zsh", include_str!("../../../assets/shell/vif.zsh")),
    ("vif.bash", include_str!("../../../assets/shell/vif.bash")),
];

/// The shell for a new pane and its arguments, with `env` set up so it loads the integration
/// where it can; None leaves the choice to the user's login shell.
pub fn prepare(config: &Config, env: &mut HashMap<String, String>) -> Option<(String, Vec<String>)> {
    let term = &config.terminal;
    let configured = term.shell.clone().map(|program| (program, term.args.clone()));
    if !term.shell_integration {
        return configured;
    }
    let program = term.shell.clone().or_else(|| std::env::var("SHELL").ok())?;
    let Some(dir) = scripts() else {
        return configured;
    };
    match program.rsplit('/').next() {
        Some("zsh") => {
            if let Ok(own) = std::env::var("ZDOTDIR") {
                env.insert("VIF_ZDOTDIR".to_owned(), own);
            }
            env.insert("ZDOTDIR".to_owned(), dir.join("zsh").display().to_string());
            env.insert("VIF_SHELL_INTEGRATION".to_owned(), dir.display().to_string());
            configured
        }
        // A macOS login shell reads .bash_profile, which --rcfile can't stand in for.
        Some("bash") if term.args.is_empty() && (term.shell.is_some() || !cfg!(target_os = "macos")) => {
            let rc = dir.join("vif.bash").display().to_string();
            Some((program, vec!["--rcfile".to_owned(), rc]))
        }
        _ => configured,
    }
}

/// Writes the scripts out once per run, where the shells can read them.
fn scripts() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = crate::terminfo::data_dir().join("shell");
        for (name, text) in FILES {
            let path = dir.join(name);
            if fs::read_to_string(&path).is_ok_and(|t| t == text) {
                continue;
            }
            fs::create_dir_all(path.parent()?).ok()?;
            fs::write(&path, text).ok()?;
        }
        Some(dir)
    })
    .clone()
}
