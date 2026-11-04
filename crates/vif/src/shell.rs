//! Shell integration: the shell marks its prompts, commands and output (OSC 133) and plays the
//! greeting, without the user touching their rc files. zsh comes in through ZDOTDIR, bash
//! through --rcfile, fish through a vendor config dir; fish 4 marks prompts on its own.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use vif_config::Config;

const FILES: [(&str, &str); 4] = [
    ("zsh/.zshenv", include_str!("../../../assets/shell/zshenv")),
    ("zsh/vif.zsh", include_str!("../../../assets/shell/vif.zsh")),
    ("vif.bash", include_str!("../../../assets/shell/vif.bash")),
    (
        "xdg/fish/vendor_conf.d/vif.fish",
        include_str!("../../../assets/shell/vif.fish"),
    ),
];

/// What every shell vif starts is told about the terminal it's in: TERM and its terminfo.
pub fn env(term: &(String, Option<PathBuf>)) -> HashMap<String, String> {
    let mut env = HashMap::from([
        ("TERM".to_owned(), term.0.clone()),
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ("TERM_PROGRAM".to_owned(), "vif".to_owned()),
        ("TERM_PROGRAM_VERSION".to_owned(), env!("CARGO_PKG_VERSION").to_owned()),
    ]);
    if let Some(dir) = &term.1 {
        env.insert("TERMINFO".to_owned(), dir.display().to_string());
    }
    env
}

/// The user's shell by name, like `zsh`.
pub fn name() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| s.rsplit('/').next().map(str::to_owned))
        .unwrap_or_else(|| "sh".to_owned())
}

/// The shell for a new pane and its arguments, with `env` set up so it loads the integration
/// where it can; None leaves the choice to the user's login shell. A `VIF_GREET` already in
/// `env` asks for the greeting, and goes if this shell can't play it.
pub fn prepare(config: &Config, env: &mut HashMap<String, String>) -> Option<(String, Vec<String>)> {
    let term = &config.terminal;
    let configured = term.shell.clone().map(|program| (program, term.args.clone()));
    let greet = env.contains_key("VIF_GREET");
    let program = term.shell.clone().or_else(|| std::env::var("SHELL").ok());
    let dir = (term.shell_integration || greet).then(scripts).flatten();
    let (Some(program), Some(dir)) = (program, dir) else {
        env.remove("VIF_GREET");
        return configured;
    };
    if term.shell_integration {
        env.insert("VIF_MARKS".to_owned(), "1".to_owned());
    }
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
        Some("fish") if greet => {
            let xdg = dir.join("xdg").display().to_string();
            let own = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
            env.insert("XDG_DATA_DIRS".to_owned(), format!("{xdg}:{own}"));
            env.insert("VIF_FISH_DIR".to_owned(), xdg);
            configured
        }
        _ => {
            env.remove("VIF_GREET");
            env.remove("VIF_MARKS");
            configured
        }
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
