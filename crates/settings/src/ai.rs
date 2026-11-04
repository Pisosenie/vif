//! The AI page: which model answers, where, and how much of a command's output it may see.

use iced::widget::{column, slider, text, toggler};
use iced::{Element, Task};

use crate::{Message, Settings, field, ranged, style};

#[derive(Clone, Debug)]
pub enum Change {
    Enabled(bool),
    Send(f32),
}

impl Settings {
    pub(crate) fn ai(&mut self, change: Change) -> Task<Message> {
        match change {
            Change::Enabled(v) => self.edit(|e| e.set("ai", "enabled", v)),
            Change::Send(v) => self.edit(|e| e.set("ai", "send", (v / 500.0).round() as i64 * 500)),
        }
        Task::none()
    }

    pub(crate) fn ai_page(&self) -> Element<'_, Message> {
        let ai = &self.config.ai;
        let accent = self.accent();
        column![
            field(
                "Ask a model",
                toggler(ai.enabled)
                    .on_toggle(|v| Message::Ai(Change::Enabled(v)))
                    .style(style::toggle(accent))
                    .into(),
                Some("off until you turn it on; nothing is sent before"),
            ),
            field(
                "Where",
                self.typed("ai.url", ai.url.clone(), "http://localhost:11434/v1", 320.0),
                Some("any OpenAI-compatible API; Ollama's on this machine by default"),
            ),
            field(
                "Model",
                self.typed("ai.model", ai.model.clone(), "qwen2.5-coder:7b", 320.0),
                Some("as the API names it, like `ollama list` shows"),
            ),
            field(
                "Key from",
                self.typed("ai.key_env", ai.key_env.clone(), "OPENAI_API_KEY", 320.0),
                Some("the environment variable with the API key, if it needs one"),
            ),
            field(
                "Output it sees",
                ranged(
                    slider(500.0f32..=20000.0, ai.send as f32, |v| Message::Ai(Change::Send(v))).step(500.0f32),
                    format!("{} characters", ai.send),
                    accent
                ),
                Some("the end of a failed command's output, where errors are"),
            ),
            text(
                "In block mode, e asks why a failed command failed; ⌘I (ctrl+shift+i on Linux) turns the words \
                 at the prompt into a command. Tokens, keys and passwords are taken out of anything sent, and \
                 a suggested command is only typed in, never run."
            )
            .size(12)
            .color(style::faint()),
        ]
        .spacing(18)
        .into()
    }
}
