//! Finding a setting by a few letters: every key vif.toml knows and every page, matched on
//! all the words typed, each found by the page it's on.

use crate::Page;

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub page: Page,
    /// The key as vif.toml spells it, or the page's title.
    pub label: String,
}

/// The page a section of vif.toml is set on.
fn page_of(section: &str) -> Option<Page> {
    match section {
        "panes" => Some(Page::Tabs),
        "badges" | "hints" => Some(Page::Terminal),
        "theme" => Some(Page::Themes),
        "background" => Some(Page::Effects),
        _ => Page::ALL.into_iter().find(|p| p.section() == section),
    }
}

/// What fits `query`, pages first, at most `limit`.
pub fn find(query: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = query
        .split(|c: char| c.is_whitespace() || c == '.' || c == '_')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        return Vec::new();
    }
    let fits = |text: &str| {
        let text = text.to_lowercase().replace('_', " ");
        words.iter().all(|w| text.contains(w.as_str()))
    };
    let pages = Page::ALL
        .into_iter()
        .filter(|p| fits(&format!("{} {}", p.title(), p.subtitle())))
        .map(|page| Hit {
            page,
            label: page.title().to_owned(),
        });
    let keys = vif_config::known().into_iter().filter_map(|key| {
        let section = key.split_once('.').map_or(key.as_str(), |(s, _)| s);
        let page = page_of(section)?;
        fits(&format!("{key} {}", page.title())).then_some(Hit { page, label: key })
    });
    pages.chain(keys).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_pages_by_any_of_their_words() {
        let hits = find("opacity", 10);
        assert_eq!(hits[0].page, Page::Window);
        assert_eq!(hits[0].label, "window.opacity");
        assert!(find("font size", 10).iter().any(|h| h.label == "font.size"));
        assert_eq!(find("quick", 10)[0].label, "Quick terminal");
        assert!(find("alphabet", 10).iter().all(|h| h.page == Page::Terminal));
        assert!(find("   ", 10).is_empty());
        assert!(find("nothing like this", 10).is_empty());
    }

    #[test]
    fn every_key_has_a_page() {
        for key in vif_config::known() {
            let section = key.split_once('.').map_or(key.as_str(), |(s, _)| s);
            assert!(page_of(section).is_some(), "{key}");
        }
    }
}
