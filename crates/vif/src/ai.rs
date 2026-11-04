//! Questions to a language model: why a command failed, and what command a few words mean. It
//! goes through curl, so vif carries no HTTP or TLS of its own, to any OpenAI-compatible API:
//! Ollama on this machine by default. Secrets are taken out before anything is sent, and the
//! request, key included, reaches curl on its stdin, never on a command line `ps` would show.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use vif_config::Ai;

const EXPLAIN: &str = "You are inside a terminal. The user ran a command and it failed. In two or \
three short sentences, say why. If a different command fixes it, end with that command alone in a \
```sh block. No greeting, no headings.";

/// What to ask.
pub enum Ask {
    /// Why this command failed.
    Explain {
        command: String,
        exit: i32,
        cwd: String,
        output: String,
    },
    /// The command these words mean.
    Command { words: String, cwd: String },
}

/// What comes back, a piece at a time.
#[derive(Debug, PartialEq)]
pub enum Answer {
    Text(String),
    Done,
    Failed(String),
}

/// The system's words and the user's for `ask`, with at most `send` characters of output, the end
/// of it, where errors are.
fn messages(ask: &Ask, send: usize) -> (String, String) {
    let os = std::env::consts::OS;
    match ask {
        Ask::Explain {
            command,
            exit,
            cwd,
            output,
        } => {
            let skip = output.chars().count().saturating_sub(send);
            let tail: String = output.chars().skip(skip).collect();
            let user = format!(
                "{os}, {shell}, in {cwd}\n$ {command}\nexit {exit}\n\n{}",
                crate::secrets::hide(&tail),
                shell = crate::shell::name(),
                command = crate::secrets::hide(command),
            );
            (EXPLAIN.to_owned(), user)
        }
        Ask::Command { words, cwd } => {
            let system = format!(
                "Turn the user's words into one {} command for {os}, run in {cwd}. Reply with the \
                 command only: one line, no explanation, no backticks.",
                crate::shell::name()
            );
            (system, crate::secrets::hide(words))
        }
    }
}

/// curl's config for the request: given on stdin, so neither the key nor the question shows in
/// the list of processes.
fn curl_config(ai: &Ai, key: Option<&str>, body: &str) -> String {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut out = format!(
        "url = {}\nheader = \"Content-Type: application/json\"\ndata-binary = {}\n",
        quote(&format!("{}/chat/completions", ai.url)),
        quote(body)
    );
    if let Some(key) = key {
        out.push_str(&format!(
            "header = {}\n",
            quote(&format!("Authorization: Bearer {key}"))
        ));
    }
    out
}

/// The text in one line of a streamed answer.
fn piece(line: &str) -> Option<Answer> {
    let data = line.strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return Some(Answer::Done);
    }
    let value: Value = serde_json::from_str(data).ok()?;
    let text = value["choices"][0]["delta"]["content"].as_str()?;
    Some(Answer::Text(text.to_owned()))
}

/// Asks, on a thread of its own; `on` hears each piece of the answer as it comes, then Done or
/// Failed.
pub fn ask(ai: &Ai, ask: Ask, mut on: impl FnMut(Answer) + Send + 'static) {
    let key = match ai.key_env.as_str() {
        "" => None,
        name => match std::env::var(name) {
            Ok(key) => Some(key),
            Err(_) => return on(Answer::Failed(format!("${name} isn't set; it should hold the API key"))),
        },
    };
    let (system, user) = messages(&ask, ai.send as usize);
    let body = json!({
        "model": ai.model,
        "stream": true,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
    })
    .to_string();
    let config = curl_config(ai, key.as_deref(), &body);
    std::thread::spawn(move || {
        let spawned = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--no-buffer",
                "--fail-with-body",
                "--config",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(err) => return on(Answer::Failed(format!("can't run curl: {err}"))),
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(config.as_bytes());
        }
        let mut body = String::new();
        let mut done = false;
        if let Some(out) = child.stdout.take() {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                match piece(&line) {
                    Some(Answer::Done) => done = true,
                    Some(answer) => on(answer),
                    None => body.push_str(&line),
                }
            }
        }
        let mut errors = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut errors);
        }
        let ok = child.wait().is_ok_and(|s| s.success());
        if ok || done {
            on(Answer::Done);
        } else {
            on(Answer::Failed(failure(&errors, &body)));
        }
    });
}

/// Asks, and hands `done` the whole answer once it's all come, or why it didn't.
pub fn ask_whole(ai: &Ai, question: Ask, done: impl FnOnce(Result<String, String>) + Send + 'static) {
    let mut text = String::new();
    let mut done = Some(done);
    ask(ai, question, move |answer| {
        let result = match answer {
            Answer::Text(piece) => return text.push_str(&piece),
            Answer::Done => Ok(std::mem::take(&mut text)),
            Answer::Failed(why) => Err(why),
        };
        if let Some(done) = done.take() {
            done(result);
        }
    });
}

/// What went wrong, the way the API or curl said it.
fn failure(errors: &str, body: &str) -> String {
    let said = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        v["error"]["message"]
            .as_str()
            .or(v["error"].as_str())
            .map(str::to_owned)
    });
    said.or_else(|| errors.lines().last().map(|l| l.trim_start_matches("curl: ").to_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "no answer".to_owned())
}

/// An answer's lines, each marked when it's inside a ``` block; the fence lines themselves go.
pub fn lines(text: &str) -> Vec<(&str, bool)> {
    let mut code = false;
    text.lines()
        .filter_map(|line| {
            if line.trim_start().starts_with("```") {
                code = !code;
                return None;
            }
            Some((line, code))
        })
        .collect()
}

/// The command at the end of an answer: its last fenced block.
pub fn fenced(text: &str) -> Option<String> {
    let lines = lines(text);
    let end = lines.iter().rposition(|(_, code)| *code)? + 1;
    let start = lines[..end].iter().rposition(|(_, code)| !*code).map_or(0, |i| i + 1);
    let block: Vec<&str> = lines[start..end].iter().map(|(l, _)| *l).collect();
    Some(block.join("\n").trim().to_owned()).filter(|b| !b.is_empty())
}

/// An answer to Command as one line to type: fences and backticks gone.
pub fn one_line(text: &str) -> String {
    let text = fenced(text).unwrap_or_else(|| text.to_owned());
    text.lines()
        .map(|l| l.trim().trim_matches('`').trim_start_matches("$ "))
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ai() -> Ai {
        Ai {
            enabled: true,
            url: "http://localhost:11434/v1".to_owned(),
            model: "m".to_owned(),
            key_env: String::new(),
            send: 20,
        }
    }

    #[test]
    fn streamed_pieces_and_the_end() {
        let line = r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#;
        assert_eq!(piece(line), Some(Answer::Text("Hel".to_owned())));
        assert_eq!(piece("data: [DONE]"), Some(Answer::Done));
        assert_eq!(piece(""), None);
        assert_eq!(piece(r#"data: {"choices":[{"delta":{}}]}"#), None);
    }

    #[test]
    fn the_question_sends_the_end_of_the_output_without_secrets() {
        let ask = Ask::Explain {
            command: "deploy --token=abc".to_owned(),
            exit: 1,
            cwd: "/srv".to_owned(),
            output: format!("{}\nAPI_KEY=hunter2 error: refused", "x".repeat(100)),
        };
        let (system, user) = messages(&ask, 30);
        assert!(system.contains("failed"));
        assert!(user.contains("$ deploy") && user.contains("exit 1") && user.contains("error: refused"));
        assert!(!user.contains("hunter2") && !user.contains(&"x".repeat(50)), "{user}");
    }

    #[test]
    fn curl_reads_the_key_from_stdin_with_quotes_kept() {
        let config = curl_config(&ai(), Some("sk-1"), r#"{"a":"say \"hi\"\n"}"#);
        assert!(config.contains(r#"url = "http://localhost:11434/v1/chat/completions""#));
        assert!(
            config.contains(r#"data-binary = "{\"a\":\"say \\\"hi\\\"\\n\"}""#),
            "{config}"
        );
        assert!(config.contains(r#"header = "Authorization: Bearer sk-1""#));
        assert!(!curl_config(&ai(), None, "{}").contains("Authorization"));
    }

    #[test]
    fn commands_come_out_of_answers() {
        let answer = "The folder doesn't exist.\n```sh\nmkdir -p build && cd build\n```";
        assert_eq!(fenced(answer).as_deref(), Some("mkdir -p build && cd build"));
        assert_eq!(fenced("no command here"), None);
        assert_eq!(one_line("`ls -la`"), "ls -la");
        assert_eq!(one_line("```bash\nfind . -size +100M\n```"), "find . -size +100M");
        assert_eq!(one_line("$ du -sh *\n"), "du -sh *");
    }

    #[test]
    fn asks_a_server_and_hears_the_answer_as_it_comes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            while !String::from_utf8_lossy(&request).contains("\"stream\":true") {
                let n = stream.read(&mut buf).unwrap();
                request.extend_from_slice(&buf[..n]);
            }
            let reply = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
                data: {\"choices\":[{\"delta\":{\"content\":\"ls \"}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"content\":\"-la\"}}]}\n\n\
                data: [DONE]\n\n";
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        let ai = Ai {
            url: format!("http://127.0.0.1:{port}/v1"),
            key_env: "PATH".to_owned(),
            ..ai()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        ask(
            &ai,
            Ask::Command {
                words: "list all".to_owned(),
                cwd: "/".to_owned(),
            },
            move |a| {
                let _ = tx.send(a);
            },
        );
        let mut got = Vec::new();
        while let Ok(answer) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            let done = answer == Answer::Done;
            got.push(answer);
            if done {
                break;
            }
        }
        assert_eq!(
            got,
            vec![Answer::Text("ls ".into()), Answer::Text("-la".into()), Answer::Done]
        );
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions"), "{request}");
        assert!(request.contains(&format!("Bearer {}", std::env::var("PATH").unwrap())));
        assert!(request.contains("\"model\":\"m\"") && request.contains("list all"));
    }

    #[test]
    fn a_missing_key_says_which() {
        let (tx, rx) = std::sync::mpsc::channel();
        let ai = Ai {
            key_env: "VIF_TEST_NO_SUCH_KEY".to_owned(),
            ..ai()
        };
        ask(
            &ai,
            Ask::Command {
                words: "x".to_owned(),
                cwd: "/".to_owned(),
            },
            move |a| {
                let _ = tx.send(a);
            },
        );
        let got = rx.recv().unwrap();
        assert!(matches!(got, Answer::Failed(m) if m.contains("VIF_TEST_NO_SUCH_KEY")));
    }
}
