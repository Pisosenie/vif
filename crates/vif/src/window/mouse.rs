//! The mouse: selecting, links under the pointer, clicks on tabs, wheel scrolling and reporting
//! all of it to programs that ask.

use super::*;

impl Win {
    pub(super) fn pane_at(&self, x: f32, y: f32) -> Option<PaneId> {
        self.areas.iter().find(|(_, a)| a.contains(x, y)).map(|(id, _)| *id)
    }

    pub(super) fn link_modifier(&self) -> bool {
        let s = self.mods.state();
        if cfg!(target_os = "macos") {
            s.super_key()
        } else {
            s.control_key()
        }
    }

    pub(super) fn hover(&mut self) {
        let (x, y) = self.mouse;
        let m = self.renderer.metrics();
        let found = self.pane_at(x, y).filter(|_| self.link_modifier()).and_then(|id| {
            let pane = self.panes.get(&id)?;
            let (col, line, _) = pane.cell_at(x, y, m);
            if let Some((key, uri)) = pane.link_at(col, line) {
                return Some(Link {
                    pane: id,
                    key,
                    uri,
                    span: None,
                    target: None,
                });
            }
            if let Some((start, end, uri)) = pane.url_at(col, line) {
                return Some(Link {
                    pane: id,
                    key: format!("url:{line}:{start}"),
                    uri,
                    span: Some((line, start, end)),
                    target: None,
                });
            }
            let (start, end, target) = pane.path_at(col, line)?;
            Some(Link {
                pane: id,
                key: format!("path:{line}:{start}"),
                uri: target.text.clone(),
                span: Some((line, start, end)),
                target: Some(target),
            })
        });
        if found == self.link {
            return;
        }
        let osc8 = |l: &Option<Link>| l.as_ref().is_some_and(|l| l.span.is_none());
        let rows = osc8(&found) || osc8(&self.link);
        self.link = found;
        self.window.set_cursor(if self.link.is_some() {
            CursorIcon::Pointer
        } else {
            CursorIcon::Text
        });
        if rows {
            self.invalidate_all();
        } else {
            self.window.request_redraw();
        }
    }

    pub(super) fn mouse_moved(&mut self, position: PhysicalPosition<f64>) {
        self.mouse = (position.x as f32, position.y as f32);
        self.window.set_cursor_visible(true);
        self.hover();
        let (x, y) = self.mouse;
        let m = self.renderer.metrics();
        let mods = self.mods.state();
        if let Some((id, button)) = self.pressed
            && let Some(pane) = self.panes.get_mut(&id)
        {
            let (col, line, side) = pane.cell_at(x, y, m);
            let mode = pane.mode();
            if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) && !mods.shift_key() {
                let code = 32 + button_code(button);
                if let Some(bytes) = input::mouse(code, col, line, true, mods, mode) {
                    pane.write(bytes);
                }
            } else if pane.selecting {
                pane.extend_selection(col, line, side);
                self.window.request_redraw();
            }
        } else if let Some(id) = self.pane_at(x, y)
            && let Some(pane) = self.panes.get(&id)
            && pane.mode().contains(TermMode::MOUSE_MOTION)
        {
            let (col, line, _) = pane.cell_at(x, y, m);
            if let Some(bytes) = input::mouse(35, col, line, true, mods, pane.mode()) {
                pane.write(bytes);
            }
        }
    }

    pub(super) fn mouse_button(&mut self, state: ElementState, button: MouseButton, shared: &Shared) {
        let (x, y) = self.mouse;
        let m = self.renderer.metrics();
        let mods = self.mods.state();
        if state == ElementState::Pressed && button == MouseButton::Left {
            if self.block_click(x, y, shared) {
                return;
            }
            if let Some(notice) = &self.notice
                && notice.area.contains(x, y)
            {
                self.notice = None;
                self.window.request_redraw();
                return;
            }
            if self.plus_hit.is_some_and(|a| a.contains(x, y)) {
                if let Err(err) = self.new_tab(shared) {
                    log::warn!("can't open a tab: {err}");
                }
                return;
            }
            if let Some((index, _)) = self.tab_hits.iter().find(|(_, a)| a.contains(x, y)) {
                self.active = *index;
                self.relayout();
                self.update_title();
                return;
            }
            if y < self.content().y - pad(&self.config, self.scale()).1 {
                let _ = self.window.drag_window();
                return;
            }
        }
        let Some(id) = self.pressed.map(|p| p.0).or_else(|| self.pane_at(x, y)) else {
            return;
        };
        if state == ElementState::Pressed {
            self.focus(id);
        }
        if state == ElementState::Pressed
            && button == MouseButton::Left
            && let Some(link) = self.link.clone()
        {
            match link.target {
                Some(target) => {
                    let cwd = self
                        .panes
                        .get(&link.pane)
                        .and_then(|p| p.last_location())
                        .and_then(|p| p.cwd);
                    self.open_target(target, cwd, shared);
                }
                None => {
                    let _ = open::that_detached(link.uri);
                }
            }
            return;
        }
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        let (col, line, side) = pane.cell_at(x, y, m);
        let mode = pane.mode();
        let reporting = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift_key();
        match state {
            ElementState::Pressed => {
                self.pressed = Some((id, button));
                if reporting {
                    if let Some(bytes) = input::mouse(button_code(button), col, line, true, mods, mode) {
                        pane.write(bytes);
                    }
                    return;
                }
                if button == MouseButton::Left
                    && let Some(fold) = pane.fold_at(line)
                {
                    pane.toggle_fold(fold);
                    pane.invalidate();
                    self.pressed = None;
                    self.window.request_redraw();
                    return;
                }
                if button == MouseButton::Left {
                    let now = Instant::now();
                    let (at, count, pos) = self.clicks;
                    let near = (pos.0 - x).abs() < m.cell_width && (pos.1 - y).abs() < m.cell_height;
                    let count = if now - at < Duration::from_millis(400) && near {
                        count % 3 + 1
                    } else {
                        1
                    };
                    self.clicks = (now, count, (x, y));
                    pane.select(col, line, side, count, mods.alt_key());
                    self.window.request_redraw();
                }
                if button == MouseButton::Middle
                    && let Some(text) = self.clipboard.get_primary()
                {
                    self.paste(text);
                }
            }
            ElementState::Released => {
                self.pressed = None;
                if reporting {
                    if let Some(bytes) = input::mouse(button_code(button), col, line, false, mods, mode) {
                        pane.write(bytes);
                    }
                    return;
                }
                pane.selecting = false;
                if button == MouseButton::Left
                    && let Some(text) = pane.selection_text()
                {
                    self.clipboard.set_primary(text);
                }
            }
        }
    }

    pub(super) fn wheel(&mut self, delta: MouseScrollDelta) {
        let (x, y) = self.mouse;
        let m = self.renderer.metrics();
        let Some(id) = self.pane_at(x, y) else {
            return;
        };
        let mods = self.mods.state();
        let Some(pane) = self.panes.get_mut(&id) else {
            return;
        };
        let pixels = match delta {
            MouseScrollDelta::LineDelta(_, lines) => lines * m.cell_height,
            MouseScrollDelta::PixelDelta(p) => p.y as f32,
        };
        pane.scroll += pixels;
        let lines = (pane.scroll / m.cell_height).trunc() as i32;
        if lines == 0 {
            return;
        }
        pane.scroll -= lines as f32 * m.cell_height;
        let mode = pane.mode();
        let (col, line, _) = pane.cell_at(x, y, m);
        if mode.intersects(TermMode::MOUSE_MODE) && !mods.shift_key() {
            let code = if lines > 0 { 64 } else { 65 };
            for _ in 0..lines.unsigned_abs().min(10) {
                if let Some(bytes) = input::mouse(code, col, line, true, mods, mode) {
                    pane.write(bytes);
                }
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && !mods.shift_key() {
            let key: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let bytes = key.repeat(lines.unsigned_abs() as usize);
            pane.write(bytes);
        } else {
            pane.scroll(Scroll::Delta(lines));
            self.window.request_redraw();
        }
    }
}

fn button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        _ => 0,
    }
}
