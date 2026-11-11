//! Secrets taken out of what the archive keeps: `KEY=value` where the name says it's a secret,
//! passwords in URLs like `postgres://user:pass@host`, tokens in the shapes the big services
//! give them, bearer headers, JWTs and private keys.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

const HIDDEN: &str = "••••";

static NAMED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b([A-Z0-9_.-]*(?:TOKEN|SECRET|PASSWORD|PASSWD|PASSPHRASE|API_?KEY|ACCESS_?KEY|PRIVATE_?KEY|CREDENTIALS?)[A-Z0-9_.-]*)(\s*[=:]\s*)("[^"\n]*"|'[^'\n]*'|[^\s"',;]+)"#,
    )
    .expect("pattern")
});

static SHAPED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}",
        r"|sk-[A-Za-z0-9_-]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35}",
        r"|eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,})",
        r"|(?i:bearer)\s+[A-Za-z0-9._~+/=-]{16,}",
    ))
    .expect("pattern")
});

/// The password in `scheme://user:password@host`; the user may be empty, as in `redis://:pw@h`.
static IN_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b([a-z][a-z0-9+.-]*://[^\s:/@]*:)[^\s/@]+@").expect("pattern"));

static PEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z)").expect("pattern")
});

/// `text` with every secret it shows replaced by dots.
pub fn hide(text: &str) -> String {
    let text: Cow<str> = PEM.replace_all(text, "-----PRIVATE KEY HIDDEN-----");
    let text = IN_URL.replace_all(&text, format!("${{1}}{HIDDEN}@"));
    let text = NAMED.replace_all(&text, format!("${{1}}${{2}}{HIDDEN}"));
    SHAPED.replace_all(&text, HIDDEN).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_values_and_known_tokens_go() {
        let out = hide("export GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123\nDB_PASSWORD: 'hunter2 two'");
        assert_eq!(out, "export GITHUB_TOKEN=••••\nDB_PASSWORD: ••••");
        assert_eq!(
            hide("curl -H 'Authorization: Bearer abcdefghijklmnop1234' x"),
            "curl -H 'Authorization: ••••' x"
        );
        assert_eq!(hide("key AKIAABCDEFGHIJKLMNOP here"), "key •••• here");
        let pem = "a\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blb\n-----END OPENSSH PRIVATE KEY-----\nz";
        assert_eq!(hide(pem), "a\n-----PRIVATE KEY HIDDEN-----\nz");
    }

    #[test]
    fn passwords_in_urls_go() {
        assert_eq!(
            hide("psql postgres://app:s3cr3t-pw@db.internal:5432/main"),
            "psql postgres://app:••••@db.internal:5432/main"
        );
        assert_eq!(
            hide("REDIS=redis://:hunter2@cache:6379"),
            "REDIS=redis://:••••@cache:6379"
        );
        assert_eq!(
            hide("git clone https://bot:glpat-abc123@gitlab.com/x.git"),
            "git clone https://bot:••••@gitlab.com/x.git"
        );
        // A port isn't a password, and an address without one is left alone.
        assert_eq!(
            hide("open http://localhost:8080/admin"),
            "open http://localhost:8080/admin"
        );
        assert_eq!(hide("ssh://git@github.com/x"), "ssh://git@github.com/x");
    }

    #[test]
    fn ordinary_output_stays() {
        let text = "cargo build --release\n   Compiling vif v0.1.2\ntoken count: 12 words, sk-short";
        assert_eq!(hide("ls -la ~/keys"), "ls -la ~/keys");
        assert_eq!(
            hide(text),
            "cargo build --release\n   Compiling vif v0.1.2\ntoken count: 12 words, sk-short"
        );
    }
}
