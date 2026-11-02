use std::fmt;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub line: usize,
    pub span: Range<usize>,
    pub message: String,
    pub hint: Option<String>,
    pub fix: Option<Fix>,
}

/// A text replacement that resolves the diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub span: Range<usize>,
    pub text: String,
}

impl Diagnostic {
    pub(crate) fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            span: 0..0,
            message: message.into(),
            hint: None,
            fix: None,
        }
    }

    pub(crate) fn at(mut self, span: Range<usize>) -> Self {
        self.span = span;
        self
    }

    pub(crate) fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub(crate) fn fix(mut self, span: Range<usize>, text: impl Into<String>) -> Self {
        self.fix = Some(Fix {
            span,
            text: text.into(),
        });
        self
    }
}

impl Fix {
    pub fn apply(&self, src: &str) -> Option<String> {
        let span = self.span.clone();
        if span.end > src.len() || !src.is_char_boundary(span.start) || !src.is_char_boundary(span.end) {
            return None;
        }
        let mut out = String::with_capacity(src.len() + self.text.len());
        out.push_str(&src[..span.start]);
        out.push_str(&self.text);
        out.push_str(&src[span.end..]);
        Some(out)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, ", {hint}")?;
        }
        Ok(())
    }
}

/// Renders diagnostics the way `vif check` prints them.
pub fn report(src: &str, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = src.lines().collect();
    for d in diagnostics {
        let Some(text) = d.line.checked_sub(1).and_then(|i| lines.get(i)) else {
            out.push_str(&format!("{d}\n\n"));
            continue;
        };
        let start = line_start(src, d.line);
        let col = d.span.start.saturating_sub(start).min(text.len());
        let width = (d.span.end.saturating_sub(d.span.start)).clamp(1, text.len().saturating_sub(col).max(1));
        let pad = " ".repeat(text[..col].chars().count());
        let marks = "^".repeat(text[col..(col + width).min(text.len())].chars().count().max(1));
        out.push_str(&format!("{:>4} │ {text}\n", d.line));
        out.push_str(&format!("     │ {pad}{marks} {d}\n\n"));
    }
    out
}

fn line_start(src: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    src.match_indices('\n').nth(line - 2).map_or(src.len(), |(i, _)| i + 1)
}
