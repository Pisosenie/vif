//! `vif theme`: list, switch, start your own, install and check themes from the shell.

use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

use vif_config::{Config, Editor, themes};

pub const USAGE: &str = "       vif theme                   list themes, * marks the one in use
       vif theme use NAME          switch every window to it
       vif theme new NAME [FROM]   start your own from FROM (or the current one) and switch
       vif theme add FILE|URL      install a theme file
       vif theme check [NAME]      show problems in a theme";

pub fn run(args: &[String]) -> ExitCode {
    let arg = |i: usize| args.get(i).map(String::as_str);
    let result = match (arg(0), arg(1)) {
        (None | Some("list"), _) => list(),
        (Some("use"), Some(name)) => switch(name),
        (Some("new"), Some(name)) => create(name, arg(2)),
        (Some("add"), Some(source)) => add(source),
        (Some("check"), name) => check(name),
        _ => Err(format!("usage:\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn current() -> String {
    Config::load().0.theme
}

fn list() -> Result<(), String> {
    let now = current();
    let all = themes::all();
    let width = all.iter().map(|t| t.name.chars().count()).max().unwrap_or(0);
    for theme in all {
        let mark = if theme.name == now { '*' } else { ' ' };
        let by = if theme.author.is_empty() {
            String::new()
        } else {
            format!(" · {}", theme.author)
        };
        let place = theme.path.as_deref().map_or_else(|| "built-in".to_owned(), tilde);
        println!("{mark} {:width$}  {}{by}  ({place})", theme.name, theme.title);
    }
    println!("\nthemes of your own go in {}", tilde(&themes::dir()));
    Ok(())
}

fn switch(name: &str) -> Result<(), String> {
    if themes::find(name).is_none() {
        let names: Vec<String> = themes::all().into_iter().map(|t| t.name).collect();
        let known: Vec<&str> = names.iter().map(String::as_str).collect();
        return Err(match vif_config::closest(name, &known) {
            Some(close) => format!("there is no theme “{name}”; did you mean “{close}”?"),
            None => format!("there is no theme “{name}”; there are {}", known.join(", ")),
        });
    }
    let mut editor = Editor::open(&vif_config::path())?;
    editor.set_top("theme", name);
    editor.save().map_err(|e| e.to_string())?;
    println!("theme {name}: open windows follow");
    Ok(())
}

fn create(name: &str, from: Option<&str>) -> Result<(), String> {
    let from = from.map_or_else(current, str::to_owned);
    let path = themes::create(name, &from)?;
    switch(name)?;
    println!("{} is yours to change; windows follow as you save", tilde(&path));
    Ok(())
}

fn add(source: &str) -> Result<(), String> {
    let text = if source.starts_with("https://") || source.starts_with("http://") {
        let out = Command::new("curl")
            .args(["-fsSL", "--max-time", "20", source])
            .output()
            .map_err(|e| format!("can't run curl: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "can't download {source}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        String::from_utf8(out.stdout).map_err(|_| "that file isn't text".to_owned())?
    } else {
        fs::read_to_string(source).map_err(|e| format!("can't read {source}: {e}"))?
    };
    let stem = source
        .rsplit('/')
        .next()
        .unwrap_or(source)
        .trim_end_matches(".toml")
        .to_owned();
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if name.is_empty() {
        return Err(format!("can't make a theme name out of {source}"));
    }
    let (path, diagnostics) = themes::install(&text, &name)?;
    if !diagnostics.is_empty() {
        print!("{}", vif_config::report(&text, &diagnostics));
    }
    println!(
        "installed {} as {name}; `vif theme use {name}` switches to it",
        tilde(&path)
    );
    Ok(())
}

fn check(name: Option<&str>) -> Result<(), String> {
    let name = name.map_or_else(current, str::to_owned);
    let (theme, diagnostics) = themes::check(&name).ok_or_else(|| format!("there is no theme “{name}”"))?;
    let place = theme
        .path
        .as_deref()
        .map_or_else(|| format!("{name} (built-in)"), tilde);
    println!("{place}\n");
    if diagnostics.is_empty() {
        println!("all good");
        return Ok(());
    }
    print!("{}", vif_config::report(&theme.source, &diagnostics));
    Err(format!("{} problem(s), the rest applies", diagnostics.len()))
}

fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}
