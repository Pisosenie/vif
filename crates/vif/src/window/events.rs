//! What happens to a window: its own events, its terminals', and what the clock brings.

use std::time::{Duration, Instant};

use alacritty_terminal::event::Event;
use alacritty_terminal::term::{ClipboardType, TermMode};
use alacritty_terminal::vte::ansi::Rgb as TermRgb;
use winit::event::{Ime, WindowEvent};
use winit::window::UserAttentionType;

use super::tabs::bar_shown;
use super::*;
use crate::app::UserEvent;
use crate::layout::PaneId;
use crate::marks;

impl Win {
    /// When the window next needs to wake up by itself.
    pub fn deadline(&self) -> Option<Instant> {
        let blink = self.blink.filter(|_| self.config.cursor.blink && self.focused);
        let notice = self.notice.as_ref().map(|n| n.until);
        let second = self.second.filter(|_| self.ticking());
        let places = self.places.filter(|_| self.follows_places());
        let toast = self.toast.as_ref().map(|t| t.1);
        [blink, notice, second, places, toast].into_iter().flatten().min()
    }

    /// Rules or tabs depend on where the panes are and what runs in them.
    pub(super) fn follows_places(&self) -> bool {
        let tabs = &self.config.tabs;
        let shown = bar_shown(&self.config, self.tabs.len());
        let title = ["{cwd}", "{dir}", "{branch}"].iter().any(|p| tabs.title.contains(p));
        !self.config.rules.is_empty() || (shown && (tabs.detail || title))
    }

    pub(super) fn ticking(&self) -> bool {
        self.areas
            .iter()
            .any(|(id, _)| self.panes.get(id).is_some_and(|p| p.ticking))
    }

    pub fn tick(&mut self, now: Instant) {
        if let Some(at) = self.blink
            && now >= at
            && self.config.cursor.blink
        {
            self.cursor_on = !self.cursor_on;
            self.blink = Some(now + BLINK);
            self.window.request_redraw();
        }
        if self.toast.as_ref().is_some_and(|t| now >= t.1) {
            self.toast = None;
            self.window.request_redraw();
        }
        if self.notice.as_ref().is_some_and(|n| now >= n.until) {
            self.notice = None;
            self.window.request_redraw();
        }
        if !self.follows_places() {
            self.places = None;
        } else if self.places.is_none_or(|at| now >= at) {
            self.places = Some(now + Duration::from_secs(1));
            let mut moved = false;
            for pane in self.panes.values_mut() {
                let before = pane.last_location();
                moved |= before.as_ref() != Some(&pane.location());
            }
            if moved {
                self.remember();
                self.window.request_redraw();
            }
        }
        if !self.ticking() {
            self.second = None;
        } else if self.second.is_none_or(|at| now >= at) {
            self.second = Some(now + Duration::from_secs(1));
            self.window.request_redraw();
        }
    }

    /// Keeps finished commands in the archive, and tells about long ones that ended where
    /// nobody was looking.
    pub(super) fn report_finished(&mut self, id: PaneId) {
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        let done = pane.blocks.take_done();
        if done.is_empty() {
            return;
        }
        let keep = self.config.terminal.archive_days > 0 && !pane.replaying();
        // Asked once here, for the archive and for the layout a restart reads.
        let place = pane.location();
        let after = Duration::from_secs(u64::from(self.config.terminal.notify_after));
        let seen = self.focused && self.areas.iter().any(|(p, _)| *p == id);
        for block in done {
            let Some((took, exit)) = pane.blocks.get(block).and_then(|b| b.ended) else {
                continue;
            };
            let typed = pane.command(block);
            if keep && let Some(command) = typed.as_deref().filter(|c| !c.starts_with(' ')) {
                crate::archive::record(&crate::archive::Entry {
                    at: crate::archive::now(),
                    took: took.as_secs_f32(),
                    exit,
                    cwd: place.cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                    host: place.host.clone(),
                    command: command.trim().to_owned(),
                    output: crate::archive::trimmed(&pane.output(block).unwrap_or_default()),
                });
            }
            let asked = self.notify.remove(&(id, block));
            if !asked && (after.is_zero() || seen || took < after || pane.replaying()) {
                continue;
            }
            let command = typed.map_or_else(|| "A command".to_owned(), |c| c.trim().to_owned());
            let (proxy, window) = (self.proxy.clone(), self.window.id());
            let show: crate::notify::Show = Box::new(move || {
                let _ = proxy.send_event(UserEvent::ShowBlock {
                    window,
                    pane: id,
                    block,
                });
            });
            crate::notify::send(
                &format!("{command} finished"),
                &format!("exit {exit} · {}", marks::spoken(took)),
                Some(show),
            );
            self.window
                .request_user_attention(Some(UserAttentionType::Informational));
        }
        self.remember();
    }

    pub(super) fn wake_cursor(&mut self) {
        self.cursor_on = true;
        self.blink = Some(Instant::now() + BLINK);
    }

    pub fn term_event(&mut self, pane: PaneId, event: Event) -> bool {
        match event {
            Event::Wakeup => {
                self.window.request_redraw();
                self.report_finished(pane);
            }
            Event::CursorBlinkingChange | Event::MouseCursorDirty => self.window.request_redraw(),
            Event::Title(title) => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.title = Some(title);
                }
                self.update_title();
                self.window.request_redraw();
            }
            Event::ResetTitle => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    p.title = None;
                }
                self.update_title();
            }
            Event::ClipboardStore(kind, text) => match kind {
                ClipboardType::Selection => self.clipboard.set_primary(text),
                ClipboardType::Clipboard => self.clipboard.set(text),
            },
            Event::ClipboardLoad(kind, format) => {
                let text = match kind {
                    ClipboardType::Selection => self.clipboard.get_primary().unwrap_or_default(),
                    ClipboardType::Clipboard => self.clipboard.get(),
                };
                if let Some(p) = self.panes.get(&pane) {
                    p.write(format(&text).into_bytes());
                }
            }
            Event::ColorRequest(index, format) => {
                let color = self.color_for(index);
                if let Some(p) = self.panes.get(&pane) {
                    p.write(format(color).into_bytes());
                }
            }
            Event::PtyWrite(text) => {
                if let Some(p) = self.panes.get(&pane) {
                    p.write(text.into_bytes());
                }
            }
            Event::TextAreaSizeRequest(format) => {
                if let Some(p) = self.panes.get(&pane) {
                    let m = self.renderer.metrics();
                    let size = alacritty_terminal::event::WindowSize {
                        num_lines: p.lines as u16,
                        num_cols: p.cols as u16,
                        cell_width: m.cell_width as u16,
                        cell_height: m.cell_height as u16,
                    };
                    p.write(format(size).into_bytes());
                }
            }
            Event::Bell => {
                if !self.focused {
                    self.window
                        .request_user_attention(Some(UserAttentionType::Informational));
                }
            }
            // Closing before the first frame would make `vif -e true` look faster than it is.
            Event::Exit | Event::ChildExit(_) if !self.shown => {
                self.doomed.push(pane);
                self.window.request_redraw();
            }
            Event::Exit | Event::ChildExit(_) => return self.close_pane(pane),
        }
        false
    }

    pub(super) fn color_for(&self, index: usize) -> TermRgb {
        let p = &self.scheme.palette;
        let c = match index {
            0..=15 => p.ansi[index],
            256 => p.foreground,
            257 => p.background,
            258 => p.cursor,
            _ => p.foreground,
        };
        TermRgb { r: c.r, g: c.g, b: c.b }
    }

    /// Handles a window event; returns true when the window should close.
    pub fn event(&mut self, event: WindowEvent, shared: &Shared) -> bool {
        match event {
            WindowEvent::CloseRequested => return true,
            WindowEvent::Resized(size) => {
                self.surface_config.width = size.width.max(1);
                self.surface_config.height = size.height.max(1);
                self.surface.configure(&self.device, &self.surface_config);
                self.relayout();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let scale = self.scale();
                self.renderer
                    .set_font(font_config(&self.config, scale, self.font_delta));
                self.relayout();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.wake_cursor();
                if let Some(pane) = self.focused_pane()
                    && pane.mode().contains(TermMode::FOCUS_IN_OUT)
                {
                    pane.write(if focused { &b"\x1b[I"[..] } else { &b"\x1b[O"[..] });
                }
                for pane in self.panes.values_mut() {
                    pane.invalidate();
                }
                self.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.mods = mods;
                self.hover();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                return self.key(event, shared);
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Preedit(text, _) => {
                    self.preedit = text;
                    self.window.request_redraw();
                }
                Ime::Commit(text) if self.palette.is_some() => {
                    self.preedit.clear();
                    self.palette_type(&text);
                }
                Ime::Commit(text) if self.history.is_some() => {
                    self.preedit.clear();
                    self.history_type(&text);
                }
                Ime::Commit(text) if self.sessions.is_some() => {
                    self.preedit.clear();
                    self.sessions_type(&text);
                }
                Ime::Commit(text) => {
                    self.preedit.clear();
                    let id = self.tabs.get(self.active).map(|t| t.focus);
                    match id.and_then(|id| self.panes.get_mut(&id)) {
                        Some(pane) if pane.hints.is_some() => {}
                        Some(pane) if pane.search.is_some() => {
                            pane.search_edit(|q| q.push_str(&text));
                            self.window.request_redraw();
                        }
                        Some(pane) => pane.input(text.into_bytes()),
                        None => {}
                    }
                }
                _ => {}
            },
            WindowEvent::CursorMoved { position, .. } => self.mouse_moved(position),
            WindowEvent::CursorLeft { .. } => {
                if self.link.take().is_some() {
                    self.invalidate_all();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => self.mouse_button(state, button, shared),
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            WindowEvent::RedrawRequested => {
                self.redraw();
                if self.shown {
                    for pane in std::mem::take(&mut self.doomed) {
                        if self.close_pane(pane) {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
        false
    }
}
