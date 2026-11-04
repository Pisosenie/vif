//! The model's answers in the window: why a block failed, in a panel as the words come, and a
//! command for the words at the prompt, typed in place of them once it's all there.

use vif_render::{Rect, Scene, Style};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use super::{UserEvent, Win};
use crate::ai::{self, Answer, Ask};
use crate::colors::mix;
use crate::layout::PaneId;

/// Why a block failed, as far as the answer has come.
pub(super) struct Reply {
    /// Which question this is, so a late answer to an older one is dropped.
    seq: u64,
    heading: String,
    text: String,
    failed: Option<String>,
    done: bool,
}

impl Win {
    /// Whether questions may be asked; says how to turn it on when not.
    fn ai_on(&mut self) -> bool {
        if !self.config.ai.enabled {
            self.tell(vec![
                "AI is off.".to_owned(),
                "Turn it on in Settings → AI, or with [ai] enabled = true in vif.toml.".to_owned(),
            ]);
        }
        self.config.ai.enabled
    }

    /// Asks why a finished block failed.
    pub(super) fn explain(&mut self, pane_id: PaneId, block: u32) {
        if !self.ai_on() {
            return;
        }
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let exit = pane.blocks.get(block).and_then(|b| b.ended).map_or(0, |(_, e)| e);
        let Some(command) = pane.command(block).filter(|_| exit != 0) else {
            return self.flash("Only a command that failed can be explained");
        };
        let question = Ask::Explain {
            command: command.trim().to_owned(),
            exit,
            cwd: pane.cwd().map(|c| crate::tilde(&c)).unwrap_or_default(),
            output: pane.output(block).unwrap_or_default(),
        };
        let heading = format!("Why `{}` failed · {}", command.trim(), self.config.ai.model);
        let seq = self.reply.as_ref().map_or(1, |r| r.seq + 1);
        self.reply = Some(Reply {
            seq,
            heading,
            text: String::new(),
            failed: None,
            done: false,
        });
        let (proxy, window) = (self.proxy.clone(), self.window.id());
        ai::ask(&self.config.ai, question, move |answer| {
            let _ = proxy.send_event(UserEvent::Ai { window, seq, answer });
        });
        self.window.request_redraw();
    }

    /// Turns the words at the prompt into a command, typed in their place.
    pub(super) fn command_from_words(&mut self) {
        if !self.ai_on() {
            return;
        }
        let Some((id, pane)) = self
            .tabs
            .get(self.active)
            .and_then(|t| Some((t.focus, self.panes.get(&t.focus)?)))
        else {
            return;
        };
        let words = pane.typed().unwrap_or_default();
        let words = words.trim_start_matches('#').trim().to_owned();
        if words.is_empty() {
            return self.flash("Type what the command should do, then ask");
        }
        let cwd = pane.cwd().map(|c| crate::tilde(&c)).unwrap_or_default();
        let (proxy, window) = (self.proxy.clone(), self.window.id());
        ai::ask_whole(&self.config.ai, Ask::Command { words, cwd }, move |result| {
            let _ = proxy.send_event(UserEvent::AiCommand {
                window,
                pane: id,
                result,
            });
        });
        self.flash(&format!("Asking {}…", self.config.ai.model));
    }

    /// The command for the words at `pane`'s prompt, typed in their place.
    pub fn ai_command(&mut self, pane: PaneId, result: Result<String, String>) {
        let command = match result {
            Ok(answer) => ai::one_line(&answer),
            Err(why) => return self.tell(vec!["The model didn't answer.".to_owned(), why]),
        };
        if command.is_empty() {
            return self.flash("No command came back");
        }
        if let Some(pane) = self.panes.get(&pane) {
            // ctrl-e and ctrl-u clear the line in zsh, bash and fish alike.
            pane.input(b"\x05\x15".to_vec());
        }
        self.paste(command);
    }

    /// A piece of an answer, or its end.
    pub fn ai_event(&mut self, seq: u64, answer: Answer) {
        let Some(reply) = self.reply.as_mut().filter(|r| r.seq == seq) else {
            return;
        };
        match answer {
            Answer::Text(text) => reply.text.push_str(&text),
            Answer::Failed(why) => reply.failed = Some(why),
            Answer::Done => reply.done = true,
        }
        self.window.request_redraw();
    }

    /// Keys while an answer shows: Enter types its command, c copies it, Esc closes it.
    pub(super) fn reply_key(&mut self, event: &KeyEvent) -> bool {
        let Some(reply) = &self.reply else {
            return false;
        };
        if event.state != ElementState::Pressed {
            return true;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.reply = None,
            Key::Named(NamedKey::Enter) => {
                if let Some(fix) = ai::fenced(&reply.text) {
                    self.reply = None;
                    self.paste(fix);
                }
            }
            Key::Character(c) if c == "c" => {
                let text = reply.text.clone();
                self.clipboard.set(text);
                self.flash("Answer copied");
            }
            _ => {}
        }
        self.window.request_redraw();
        true
    }

    pub(super) fn draw_reply(&mut self, scene: &mut Scene) {
        let Some(reply) = &self.reply else {
            return;
        };
        let (heading, answer, failed, done) = (
            reply.heading.clone(),
            reply.text.clone(),
            reply.failed.clone(),
            reply.done,
        );
        let m = self.renderer.metrics();
        let scale = self.scale();
        let scheme = self.scheme;
        let content = self.content();
        let pad = (14.0 * scale).round();
        let w = (content.w - 2.0 * pad).min(110.0 * m.cell_width);
        let cols = ((w - 2.0 * pad) / m.cell_width).max(10.0) as usize;
        let body = match (&failed, answer.is_empty(), done) {
            (Some(why), ..) => format!("The model didn't answer: {why}"),
            (None, true, false) => "…".to_owned(),
            (None, true, true) => "No answer came back.".to_owned(),
            (None, false, _) => answer.clone(),
        };
        let lines = shown(&body, cols);
        let fix = done.then(|| ai::fenced(&answer)).flatten();
        let most = ((content.h * 0.5) / m.cell_height) as usize;
        let last: Vec<&(String, bool)> = lines.iter().skip(lines.len().saturating_sub(most.max(3))).collect();
        let h = (last.len() + 3) as f32 * m.cell_height + 2.0 * pad;
        let (x, y) = (content.x + (content.w - w) / 2.0, content.y + content.h - h - pad);
        let panel = mix(scheme.fg(), scheme.bg(), 0.9);
        scene
            .front
            .push(Rect::fill(x, y, w, h, panel).rounded(self.radius() * 1.5));
        let mut text = Scene::default();
        let muted = mix(scheme.fg(), scheme.bg(), 0.45);
        let heading: String = heading.chars().take(cols).collect();
        let mut ty = y + pad;
        self.label(&heading, x + pad, ty, scheme.accent(), Style::Bold, &mut text);
        ty += 1.5 * m.cell_height;
        for (line, code) in last {
            let color = if *code { scheme.accent() } else { scheme.fg() };
            self.label(line, x + pad, ty, color, Style::Regular, &mut text);
            ty += m.cell_height;
        }
        let keys = match (fix.is_some(), done) {
            (true, _) => "↵ type the fix at the prompt · c copy · Esc close",
            (false, true) => "c copy · Esc close",
            (false, false) => "Esc close",
        };
        self.label(
            keys,
            x + pad,
            y + h - pad - m.cell_height,
            muted,
            Style::Regular,
            &mut text,
        );
        scene.over.extend(text.mask);
    }
}

/// The answer's lines as they show: what fences hold indented and marked as code.
fn shown(text: &str, cols: usize) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for (line, code) in ai::lines(text) {
        let room = if code { cols.saturating_sub(2).max(1) } else { cols };
        out.extend(
            wrap(line, room)
                .into_iter()
                .map(|l| if code { (format!("  {l}"), true) } else { (l, false) }),
        );
    }
    out
}

/// `text` in lines of at most `cols` characters, broken at spaces where it can be.
fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let (mut current, mut len) = (String::new(), 0);
        for word in line.split(' ') {
            let size = word.chars().count();
            if len > 0 && len + size >= cols {
                out.push(std::mem::take(&mut current));
                len = 0;
            }
            if len > 0 {
                current.push(' ');
                len += 1;
            }
            current.push_str(word);
            len += size;
            while len > cols {
                let head: String = current.chars().take(cols).collect();
                current = current.chars().skip(cols).collect();
                out.push(head);
                len -= cols;
            }
        }
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_wrap_at_spaces() {
        assert_eq!(
            wrap("the folder build is missing", 12),
            vec!["the folder", "build is", "missing"]
        );
        assert_eq!(wrap("a\n\nb", 5), vec!["a", "", "b"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn fenced_commands_show_as_code_without_fences() {
        let lines = shown("It's gone.\n```sh\nmkdir -p x\n```", 40);
        assert_eq!(
            lines,
            vec![("It's gone.".to_owned(), false), ("  mkdir -p x".to_owned(), true)]
        );
    }
}
