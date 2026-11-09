//! vif, a GPU terminal. The binary is `main` here; `fuzz` opens a few parsers to cargo-fuzz.

#![warn(clippy::undocumented_unsafe_blocks)]

mod ai;
mod app;
mod archive;
mod blocks;
mod clipboard;
mod colors;
mod facts;
mod fold;
#[doc(hidden)]
pub mod fuzz;

mod gifplay;
mod graphics;
mod greet;
mod hints;
mod input;
mod layout;
mod layouts;
mod marks;
#[cfg(target_os = "macos")]
mod menu;
mod niri;
mod notify;
mod pane;
mod private;
mod process;
mod quick;
mod remote;
mod rules;
mod screen;
mod secrets;
mod session;
mod shell;
mod terminfo;
mod theme;
mod tty;
mod wallpaper;
mod window;

use std::fs;
use std::process::ExitCode;

use vif_config::Config;
use winit::event_loop::EventLoop;

use app::{App, UserEvent};

const USAGE: &str = "usage: vif [-e command [args…]]
       vif check                   show problems in vif.toml
       vif set KEY VALUE           change one setting, like `vif set window.opacity 0.9`
       vif history [WORDS…]        commands kept with their output, by what they were or printed
       vif sessions                every shell vif keeps running, and which have no window
       vif attach NAME|ID          a window for one of them
       vif quick                   show or hide the quick terminal (bind it to a key)
       vif niri                    a fragment for niri's config: the quick terminal's place and key
       vif settings [PAGE]         the settings window, on a page like `font` or `background`
       vif --version";

pub fn main() -> ExitCode {
    app::started();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("gif") => gifplay::run(&args[1..]),
        Some("greet") => greet::run(&args[1..]),
        Some("settings") => {
            window::open_settings(args.get(1).map(String::as_str));
            ExitCode::SUCCESS
        }
        Some("niri") => {
            print!("{}", niri::fragment(&Config::load().0));
            ExitCode::SUCCESS
        }
        Some("set") if args.len() >= 3 => set(&args[1], &args[2..].join(" ")),
        Some("theme" | "themes") => theme::run(&args[1..]),
        Some("history") => history(&args[1..]),
        Some("sessions") => sessions(),
        Some("attach") if args.len() == 2 => match session::find(&args[1]) {
            Some(id) => run(None, Some(id), false),
            None => {
                eprintln!("vif: no session called {}; `vif sessions` lists them", args[1]);
                ExitCode::FAILURE
            }
        },
        Some("quick") => {
            if quick::ask() {
                return ExitCode::SUCCESS;
            }
            run(None, None, true)
        }
        Some("session-host") => session::host(&args[1..]),
        Some("ssh") => remote::ssh(&args[1..]),
        Some("remote") => remote::remote(&args[1..]),
        Some("udp-agent") => remote::udp_agent(&args[1..]),
        Some("--version" | "-V") => {
            println!("vif {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            println!(
                "{USAGE}\n{}\n{}\n{}\n{}",
                theme::USAGE,
                remote::USAGE,
                gifplay::USAGE,
                greet::USAGE
            );
            ExitCode::SUCCESS
        }
        Some("-e" | "--command") if args.len() > 1 => run(Some(args[1..].to_vec()), None, false),
        Some(_) => {
            eprintln!("{USAGE}\n{}", theme::USAGE);
            ExitCode::from(2)
        }
        None => run(None, None, false),
    }
}

fn run(command: Option<Vec<String>>, attach: Option<String>, quick: bool) -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(err) => {
            eprintln!("vif: {err}");
            return ExitCode::FAILURE;
        }
    };
    app::mark("event loop");
    let mut app = App::new(event_loop.create_proxy(), command, attach, quick);
    app::mark("config");
    match event_loop.run_app(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vif: {err}");
            ExitCode::FAILURE
        }
    }
}

/// A path with the home folder as `~`.
pub fn tilde(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn sessions() -> ExitCode {
    let all = session::all();
    if all.is_empty() {
        println!("no sessions");
        return ExitCode::SUCCESS;
    }
    for s in &all {
        let state = if s.attached { "in a window" } else { "waiting" };
        let name = s.name.as_deref().unwrap_or("-");
        println!("{:<17} {name:<16} {state:<12} {}", s.id, s.what);
    }
    println!("\n`vif attach NAME|ID` opens one; ⇧⌘S shows them in vif");
    ExitCode::SUCCESS
}

fn history(words: &[String]) -> ExitCode {
    use std::io::IsTerminal;
    let query = words.join(" ");
    let found = archive::find(&archive::lines(usize::MAX), &query, 20);
    if found.is_empty() {
        println!("nothing kept like that");
        return ExitCode::FAILURE;
    }
    let (dim, off) = if std::io::stdout().is_terminal() {
        ("\x1b[2m", "\x1b[0m")
    } else {
        ("", "")
    };
    for e in found.iter().rev() {
        let mark = if e.exit == 0 {
            "✓".to_owned()
        } else {
            format!("✗ {}", e.exit)
        };
        let place = e.host.as_deref().unwrap_or(&e.cwd);
        println!("{:>9}  {mark:<5} {}  {dim}{place}{off}", archive::ago(e.at), e.command);
        let line = archive::snippet(e, &query);
        if !line.is_empty() {
            println!("{:>9}  {dim}{line}{off}", "");
        }
    }
    ExitCode::SUCCESS
}

/// `vif set window.opacity 0.9`: writes one setting in place and says what vif made of it.
fn set(key: &str, value: &str) -> ExitCode {
    let known = vif_config::known();
    if !known.iter().any(|k| k == key) {
        let names: Vec<&str> = known.iter().map(String::as_str).collect();
        match vif_config::closest(key, &names) {
            Some(close) => eprintln!("vif: no setting {key}; did you mean {close}?"),
            None => eprintln!("vif: no setting {key}; `vif check` shows the file, the README lists them all"),
        }
        return ExitCode::from(2);
    }
    let value: toml_edit::Value = match value.parse::<toml_edit::Value>() {
        Ok(v) if !matches!(v, toml_edit::Value::String(_)) || value.starts_with('"') => v,
        _ => value.into(),
    };
    let path = vif_config::path();
    let result = vif_config::Editor::open(&path).and_then(|mut editor| {
        match key.split_once('.') {
            Some((section, name)) => editor.set(section, name, value.clone()),
            None => editor.set_top(key, value.clone()),
        }
        editor.save().map(|_| ()).map_err(|e| e.to_string())
    });
    if let Err(err) = result {
        eprintln!("vif: {err}");
        return ExitCode::FAILURE;
    }
    println!("{key} = {}", value.to_string().trim());
    check()
}

fn check() -> ExitCode {
    let path = vif_config::path();
    let src = match fs::read_to_string(&path) {
        Ok(src) => src,
        Err(_) => {
            println!("{}: not there yet, the defaults apply", path.display());
            return ExitCode::SUCCESS;
        }
    };
    let (config, diagnostics) = Config::parse(&src);
    println!("{}\n", path.display());
    let mut looks = Vec::new();
    for (name, source) in window::looks::shader_sources(&config, &mut looks) {
        if let Err(err) = vif_render::check_shader(&name, &source) {
            looks.push(err);
        }
    }
    if let Some(Err(err)) = wallpaper::load(&config.background) {
        looks.push(err);
    }
    for problem in &looks {
        println!("{problem}");
    }
    if diagnostics.is_empty() {
        if looks.is_empty() {
            println!("all good");
            return ExitCode::SUCCESS;
        }
        return ExitCode::FAILURE;
    }
    print!("{}", vif_config::report(&src, &diagnostics));
    let n = diagnostics.len();
    println!("{n} problem{}, everything else applied", if n == 1 { "" } else { "s" });
    ExitCode::FAILURE
}
