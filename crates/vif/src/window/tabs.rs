//! The tab bar: where it sits, what each tab says, and drawing it in the theme's style.

use vif_config::{Config, Decorations, TabPosition, TabShow, TabStyle};
use vif_render::chrome::{self, Bar, BarStyle, Tab};
use vif_render::{Rect, Scene, Style};

use super::Win;
use crate::colors::mix;
use crate::layout::Area;
use crate::rules::Place;

impl Win {
    /// Room above and below the panes: the tab bar's, or the quick terminal's header.
    pub(super) fn bands(&self) -> (f32, f32) {
        let scale = self.scale();
        if self.quick {
            return ((36.0 * scale).round(), 0.0);
        }
        insets(&self.config, scale, self.tabs.len().max(1))
    }

    /// The quick terminal's header: what it is, and the key that hides it.
    fn draw_quick_header(&mut self, scene: &mut Scene) {
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let (w, _) = self.size();
        let band = self.bands().0;
        let ch = m.cell_height;
        let ty = ((band - ch) / 2.0).round();
        let d = (ch * 0.36).round();
        let x = (14.0 * scale).round();
        scene
            .back
            .push(Rect::fill(x, ty + (ch - d) / 2.0, d, d, scheme.accent()).rounded(d / 2.0));
        let mut text = Scene::default();
        let mut at = x + d + 8.0 * scale;
        at += self.label("quick", at, ty, scheme.fg(), Style::Bold, &mut text) + m.cell_width;
        let muted = mix(scheme.fg(), scheme.bg(), 0.5);
        self.label("always running", at, ty, muted, Style::Regular, &mut text);
        let key = match (cfg!(target_os = "macos"), self.config.quick.key.as_str()) {
            (false, _) => "vif quick".to_owned(),
            (true, "") => "⇧⌘P Quick terminal".to_owned(),
            (true, key) => super::palette::keycaps(key),
        };
        let hint = format!("{key} hide");
        let hx = w - (hint.chars().count() as f32 * m.cell_width) - 14.0 * scale;
        self.label(&hint, hx, ty, muted, Style::Regular, &mut text);
        scene.back.push(Rect::fill(
            0.0,
            band - scale,
            w,
            scale.max(1.0),
            mix(scheme.fg(), scheme.bg(), 0.88),
        ));
        scene.over.extend(text.mask);
    }

    pub(super) fn draw_tabs(&mut self, scene: &mut Scene) {
        self.tab_hits.clear();
        self.plus_hit = None;
        if self.quick {
            return self.draw_quick_header(scene);
        }
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
        let titles: Vec<Tab> = (0..self.tabs.len()).map(|i| self.tab(i)).collect();
        let scheme = self.scheme;
        let bar = Bar {
            x: left,
            y,
            w: w - left - 8.0 * scale,
            h,
            tabs: &titles,
            plus: self.config.tabs.plus,
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
        let (hits, plus) = chrome::tab_bar(&mut self.renderer, &bar, scene);
        self.tab_hits = hits
            .into_iter()
            .enumerate()
            .map(|(i, (x, y, w, h))| (i, Area { x, y, w, h }))
            .collect();
        self.plus_hit = plus.map(|(x, y, w, h)| Area { x, y, w, h });
    }

    /// A tab as the bar shows it: the theme's `title` filled in, then what runs there or its
    /// folder, with a dot that is the accent on the tab in front and green while something runs.
    pub(super) fn tab(&mut self, index: usize) -> Tab {
        let format = self.config.tabs.title.clone();
        let id = self.tabs[index].focus;
        let Some(pane) = self.panes.get_mut(&id) else {
            return Tab {
                title: "vif".to_owned(),
                detail: String::new(),
                dot: None,
            };
        };
        let place = pane.location();
        let title = match &pane.name {
            Some(name) => name.clone(),
            None => title_text(&format, index + 1, pane.title.as_deref(), &place),
        };
        let detail = match (&place.running, &place.cwd) {
            _ if !self.config.tabs.detail => String::new(),
            (Some(running), _) => running.clone(),
            (None, Some(cwd)) if !format.contains("{cwd}") => crate::tilde(cwd),
            _ => String::new(),
        };
        // The branch follows the folder, quieter still, unless the title says it already.
        let detail = match place
            .branch
            .as_deref()
            .filter(|_| self.config.tabs.detail && !format.contains("{branch}"))
        {
            Some(branch) if detail.is_empty() => branch.to_owned(),
            Some(branch) if place.running.is_none() => format!("{detail} · {branch}"),
            _ => detail,
        };
        let detail = match (self.tabs[index].zoomed, detail.is_empty()) {
            (false, _) => detail,
            (true, true) => "zoomed".to_owned(),
            (true, false) => format!("zoomed · {detail}"),
        };
        let scheme = self.scheme;
        let dot = if place.running.is_some() {
            Some(crate::colors::rgba(scheme.palette.ansi[2], 1.0))
        } else if index == self.active {
            Some(scheme.accent())
        } else {
            Some(crate::colors::mix(scheme.fg(), scheme.bg(), 0.7))
        };
        Tab { title, detail, dot }
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

/// Fills in {index}, {title}, {cwd}, {dir} and {branch}, and drops the gaps a missing part
/// leaves. Over ssh, {dir} is the host.
pub(super) fn title_text(format: &str, index: usize, title: Option<&str>, place: &Place) -> String {
    let cwd_text = place.cwd.as_deref().map(crate::tilde);
    let dir_text = place.host.clone().or_else(|| {
        cwd_text.as_deref().map(|c| match c.rsplit('/').next() {
            Some("") => "/".to_owned(),
            Some(last) => last.to_owned(),
            None => c.to_owned(),
        })
    });
    let text = format
        .replace("{index}", &index.to_string())
        .replace("{title}", title.unwrap_or("vif"))
        .replace("{cwd}", cwd_text.as_deref().unwrap_or(""))
        .replace("{dir}", dir_text.as_deref().unwrap_or(""))
        .replace("{branch}", place.branch.as_deref().unwrap_or(""));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::title_text;
    use crate::rules::Place;
    use std::path::{Path, PathBuf};

    fn at(cwd: Option<&Path>, host: Option<&str>, branch: Option<&str>) -> Place {
        Place {
            cwd: cwd.map(Path::to_owned),
            host: host.map(str::to_owned),
            branch: branch.map(str::to_owned),
            ..Place::default()
        }
    }

    #[test]
    fn tab_titles_fill_in_and_close_gaps() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let cwd = home.join("src/vif");
        let nowhere = Place::default();
        assert_eq!(title_text("{index} {title}", 2, Some("nvim"), &nowhere), "2 nvim");
        assert_eq!(
            title_text("{index} {dir}", 1, None, &at(Some(&cwd), None, None)),
            "1 vif"
        );
        assert_eq!(
            title_text("[{index}] {cwd}", 3, None, &at(Some(&cwd), None, None)),
            "[3] ~/src/vif"
        );
        assert_eq!(title_text("{dir}", 1, None, &at(Some(&home), None, None)), "~");
        assert_eq!(title_text("{dir}", 1, None, &at(Some(Path::new("/")), None, None)), "/");
        assert_eq!(title_text("{index} {dir} {title}", 4, Some("zsh"), &nowhere), "4 zsh");
        assert_eq!(
            title_text("{dir}", 1, None, &at(Some(&cwd), Some("devbox"), None)),
            "devbox"
        );
        assert_eq!(
            title_text("{dir} {branch}", 1, None, &at(Some(&cwd), None, Some("main"))),
            "vif main"
        );
        assert_eq!(
            title_text("{dir} {branch}", 1, None, &at(Some(&cwd), None, None)),
            "vif"
        );
    }
}
