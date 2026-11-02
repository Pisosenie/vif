//! The tab bar: where it sits, what each tab says, and drawing it in the theme's style.

use std::path::PathBuf;

use vif_config::{Config, Decorations, TabPosition, TabShow, TabStyle};
use vif_render::Scene;
use vif_render::chrome::{self, Bar, BarStyle};

use super::Win;
use crate::layout::Area;

impl Win {
    pub(super) fn draw_tabs(&mut self, scene: &mut Scene) {
        self.tab_hits.clear();
        if !bar_shown(&self.config, self.tabs.len()) {
            return;
        }
        let scale = self.scale();
        let m = self.renderer.metrics();
        let (top, bottom) = insets(&self.config, scale, self.tabs.len());
        let (w, height) = self.size();
        let at_top = self.config.tabs.position == TabPosition::Top;
        let titlebar = cfg!(target_os = "macos") && self.config.window.decorations == Decorations::Transparent;
        let left = if at_top && titlebar { 78.0 * scale } else { 8.0 * scale };
        let band = if at_top { top } else { bottom };
        let h = (band - 8.0 * scale).max(m.cell_height + 4.0 * scale);
        let y = if at_top {
            ((band - h) / 2.0).round()
        } else {
            (height - band + (band - h) / 2.0).round()
        };
        let titles: Vec<String> = (0..self.tabs.len()).map(|i| self.tab_title(i)).collect();
        let scheme = self.scheme;
        let bar = Bar {
            x: left,
            y,
            w: w - left - 8.0 * scale,
            h,
            tabs: &titles,
            active: self.active,
            style: match self.config.tabs.style {
                TabStyle::Pills => BarStyle::Pills,
                TabStyle::Underline => BarStyle::Underline,
                TabStyle::Powerline => BarStyle::Powerline,
                TabStyle::Plain => BarStyle::Plain,
            },
            fg: scheme.fg(),
            bg: scheme.bg(),
            accent: scheme.accent(),
            radius: self.config.window.radius,
            scale,
        };
        let hits = chrome::tab_bar(&mut self.renderer, &bar, scene);
        self.tab_hits = hits
            .into_iter()
            .enumerate()
            .map(|(i, (x, y, w, h))| (i, Area { x, y, w, h }))
            .collect();
    }

    /// A tab's label from the theme's `title`, with the parts it asks for filled in.
    pub(super) fn tab_title(&mut self, index: usize) -> String {
        let format = self.config.tabs.title.clone();
        let wants_dir = format.contains("{cwd}") || format.contains("{dir}");
        let id = self.tabs[index].focus;
        let (title, cwd) = match self.panes.get_mut(&id) {
            Some(pane) => (pane.title.clone(), pane.location().cwd.filter(|_| wants_dir)),
            None => (None, None),
        };
        title_text(&format, index + 1, title.as_deref(), cwd.as_deref())
    }
}

pub(super) fn bar_shown(config: &Config, tabs: usize) -> bool {
    match config.tabs.show {
        TabShow::Never => false,
        TabShow::Always => true,
        TabShow::Many => tabs > 1,
    }
}

/// Room the titlebar and the tab bar take above and below the panes.
pub(super) fn insets(config: &Config, scale: f32, tabs: usize) -> (f32, f32) {
    let titlebar = cfg!(target_os = "macos") && config.window.decorations == Decorations::Transparent;
    let bar = bar_shown(config, tabs);
    let on_top = bar && config.tabs.position == TabPosition::Top;
    let top: f32 = match (titlebar, on_top) {
        (true, true) => 38.0,
        (true, false) => 28.0,
        (false, true) => 34.0,
        (false, false) => 0.0,
    };
    let bottom: f32 = if bar && !on_top { 34.0 } else { 0.0 };
    ((top * scale).round(), (bottom * scale).round())
}

/// Fills in {index}, {title}, {cwd} and {dir}, and drops the gaps a missing part leaves.
pub(super) fn title_text(format: &str, index: usize, title: Option<&str>, cwd: Option<&std::path::Path>) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let cwd_text = cwd.map(|dir| match home.as_deref().and_then(|h| dir.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    });
    let dir_text = cwd_text.as_deref().map(|c| match c.rsplit('/').next() {
        Some("") => "/".to_owned(),
        Some(last) => last.to_owned(),
        None => c.to_owned(),
    });
    let text = format
        .replace("{index}", &index.to_string())
        .replace("{title}", title.unwrap_or("vif"))
        .replace("{cwd}", cwd_text.as_deref().unwrap_or(""))
        .replace("{dir}", dir_text.as_deref().unwrap_or(""));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::title_text;
    use std::path::{Path, PathBuf};

    #[test]
    fn tab_titles_fill_in_and_close_gaps() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let cwd = home.join("src/vif");
        assert_eq!(title_text("{index} {title}", 2, Some("nvim"), None), "2 nvim");
        assert_eq!(title_text("{index} {dir}", 1, None, Some(&cwd)), "1 vif");
        assert_eq!(title_text("[{index}] {cwd}", 3, None, Some(&cwd)), "[3] ~/src/vif");
        assert_eq!(title_text("{dir}", 1, None, Some(&home)), "~");
        assert_eq!(title_text("{dir}", 1, None, Some(Path::new("/"))), "/");
        assert_eq!(title_text("{index} {dir} {title}", 4, Some("zsh"), None), "4 zsh");
    }
}
