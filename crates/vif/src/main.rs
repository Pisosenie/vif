mod app;
mod archive;
mod clipboard;
mod colors;

mod graphics;
mod hints;
mod input;
mod layout;
mod marks;
mod notify;
mod pane;
mod process;
mod rules;
mod session;
mod shell;
mod terminfo;
mod theme;
mod window;

use std::fs;
use std::process::ExitCode;

use vif_config::Config;
use winit::event_loop::EventLoop;

use app::{App, UserEvent};

const USAGE: &str = "usage: vif [-e command [args…]]
       vif check                   show problems in vif.toml
       vif history [WORDS…]        commands kept with their output, by what they were or printed
       vif sessions                shells left running by closed windows
       vif attach ID               a window for one of them
       vif --version";

fn main() -> ExitCode {
    app::started();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => check(),
        Some("theme" | "themes") => theme::run(&args[1..]),
        Some("history") => history(&args[1..]),
        Some("sessions") => sessions(),
        Some("attach") if args.len() == 2 => run(None, Some(args[1].clone())),
        Some("session-host") => session::host(&args[1..]),
        Some("--version" | "-V") => {
            println!("vif {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            println!("{USAGE}\n{}", theme::USAGE);
            ExitCode::SUCCESS
        }
        Some("-e" | "--command") if args.len() > 1 => run(Some(args[1..].to_vec()), None),
        Some(_) => {
            eprintln!("{USAGE}\n{}", theme::USAGE);
            ExitCode::from(2)
        }
        None => run(None, None),
    }
}

fn run(command: Option<Vec<String>>, attach: Option<String>) -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(err) => {
            eprintln!("vif: {err}");
            return ExitCode::FAILURE;
        }
    };
    app::mark("event loop");
    let mut app = App::new(event_loop.create_proxy(), command, attach);
    app::mark("config");
    match event_loop.run_app(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vif: {err}");
            ExitCode::FAILURE
        }
    }
}

fn sessions() -> ExitCode {
    let free = session::free();
    if free.is_empty() {
        println!("no shells waiting for a window");
    }
    for id in free {
        println!("{id}  `vif attach {id}`");
    }
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

fn check() -> ExitCode {
    let path = vif_config::path();
    let src = match fs::read_to_string(&path) {
        Ok(src) => src,
        Err(_) => {
            println!("{}: not there yet, the defaults apply", path.display());
            return ExitCode::SUCCESS;
        }
    };
    let (_, diagnostics) = Config::parse(&src);
    println!("{}\n", path.display());
    if diagnostics.is_empty() {
        println!("all good");
        return ExitCode::SUCCESS;
    }
    print!("{}", vif_config::report(&src, &diagnostics));
    let n = diagnostics.len();
    println!("{n} problem{}, everything else applied", if n == 1 { "" } else { "s" });
    ExitCode::FAILURE
}
