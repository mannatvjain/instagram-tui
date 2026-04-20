use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectThread {
    pub thread_id: String,
    pub thread_title: String,
    pub usernames: Vec<String>,
    pub last_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectMessage {
    pub user_id: String,
    pub text: String,
    pub timestamp: String,
    pub is_sender: bool,
}

pub struct InstagramClient {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl InstagramClient {
    pub fn new() -> Result<Self> {
        // Find bridge.py relative to the executable or in known locations
        let bridge_path = Self::find_bridge()?;
        let venv_python = Self::find_python(&bridge_path)?;

        let mut child = Command::new(&venv_python)
            .arg(&bridge_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start bridge at {}", bridge_path.display()))?;

        let stdout = child.stdout.take().context("failed to get bridge stdout")?;
        let reader = BufReader::new(stdout);

        Ok(Self { child, reader })
    }

    fn find_bridge() -> Result<PathBuf> {
        // Check next to the executable
        if let Ok(exe) = std::env::current_exe() {
            let dir = exe.parent().unwrap_or(std::path::Path::new("."));
            let candidate = dir.join("bridge.py");
            if candidate.exists() {
                return Ok(candidate);
            }
            // Check two levels up (target/release/inote -> project root)
            if let Some(project) = dir.parent().and_then(|p| p.parent()) {
                let candidate = project.join("bridge.py");
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
        // Check current directory
        let cwd = std::env::current_dir()?;
        let candidate = cwd.join("bridge.py");
        if candidate.exists() {
            return Ok(candidate);
        }
        // Check home directory project
        if let Some(home) = dirs_home() {
            let candidate = home.join("Developer/instagram-tui/bridge.py");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
        bail!("bridge.py not found — run from the instagram-tui project directory")
    }

    fn find_python(bridge_path: &PathBuf) -> Result<PathBuf> {
        // Look for .venv/bin/python3 relative to bridge.py
        let project_dir = bridge_path.parent().unwrap_or(std::path::Path::new("."));
        let venv_python = project_dir.join(".venv/bin/python3");
        if venv_python.exists() {
            return Ok(venv_python);
        }
        // Fall back to system python3
        Ok(PathBuf::from("python3"))
    }

    fn call(&mut self, cmd: &str, args: Value) -> Result<Value> {
        let req = json!({"cmd": cmd, "args": args});
        let stdin = self.child.stdin.as_mut().context("bridge stdin closed")?;
        writeln!(stdin, "{}", req)?;
        stdin.flush()?;

        let mut line = String::new();
        self.reader.read_line(&mut line).context("bridge read failed")?;

        if line.is_empty() {
            bail!("bridge process exited unexpectedly");
        }

        let resp: Value = serde_json::from_str(&line).context("bridge returned invalid JSON")?;

        if resp.get("ok") == Some(&Value::Bool(true)) {
            Ok(resp)
        } else {
            let err = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown bridge error");
            bail!("{}", err)
        }
    }

    pub fn login(&mut self, username: &str, password: &str) -> Result<(String, String)> {
        let resp = self.call("login", json!({
            "username": username,
            "password": password,
        }))?;
        let user = resp.get("username").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let uid = resp.get("user_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok((user, uid))
    }

    pub fn get_direct_threads(&mut self, limit: usize) -> Result<Vec<DirectThread>> {
        let resp = self.call("threads", json!({"amount": limit}))?;
        let threads: Vec<DirectThread> = serde_json::from_value(
            resp.get("threads").cloned().unwrap_or(Value::Array(vec![]))
        )?;
        Ok(threads)
    }

    pub fn get_thread_messages(&mut self, thread_id: &str, limit: usize) -> Result<(Vec<DirectMessage>, String)> {
        let resp = self.call("messages", json!({
            "thread_id": thread_id,
            "amount": limit,
        }))?;
        let messages: Vec<DirectMessage> = serde_json::from_value(
            resp.get("messages").cloned().unwrap_or(Value::Array(vec![]))
        )?;
        let title = resp.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok((messages, title))
    }

    pub fn send_dm(&mut self, thread_id: &str, text: &str) -> Result<()> {
        self.call("send_dm", json!({
            "thread_id": thread_id,
            "text": text,
        }))?;
        Ok(())
    }

    pub fn create_note(&mut self, text: &str) -> Result<String> {
        let resp = self.call("create_note", json!({"text": text}))?;
        let note_id = resp.get("note_id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
        Ok(note_id)
    }

    // ── Parsing helpers (kept for tests) ────────────────────────────────

    pub fn parse_threads_response(body: &Value) -> Vec<DirectThread> {
        let threads = body
            .pointer("/inbox/threads")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        threads.iter().map(|t| {
            let thread_id = t.get("thread_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let thread_title = t.get("thread_title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let usernames: Vec<String> = t.get("users")
                .and_then(|v| v.as_array())
                .map(|users| users.iter().filter_map(|u| u.get("username").and_then(|v| v.as_str())).map(String::from).collect())
                .unwrap_or_default();
            let last_message = t.get("items").and_then(|v| v.as_array()).and_then(|items| items.first())
                .and_then(|item| item.get("text").and_then(|v| v.as_str()).or_else(|| item.get("item_type").and_then(|v| v.as_str())))
                .unwrap_or("[media]").to_string();
            let title = if thread_title.is_empty() { usernames.join(", ") } else { thread_title };
            DirectThread { thread_id, thread_title: title, usernames, last_message }
        }).collect()
    }

    pub fn parse_messages_response(body: &Value, my_user_id: &str) -> (Vec<DirectMessage>, String) {
        let thread_title = body.pointer("/thread/thread_title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let items = body.pointer("/thread/items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let users = body.pointer("/thread/users").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let user_map: std::collections::HashMap<String, String> = users.iter().filter_map(|u| {
            let pk = u.get("pk")?.to_string().trim_matches('"').to_string();
            let name = u.get("username")?.as_str()?.to_string();
            Some((pk, name))
        }).collect();

        let messages = items.iter().map(|item| {
            let user_id = item.get("user_id").map(|v| v.to_string().trim_matches('"').to_string()).unwrap_or_default();
            let text = item.get("text").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| {
                format!("[{}]", item.get("item_type").and_then(|v| v.as_str()).unwrap_or("media"))
            });
            let timestamp = item.get("timestamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let is_sender = user_id == my_user_id;
            DirectMessage {
                user_id: user_map.get(&user_id).cloned().unwrap_or_else(|| if is_sender { "you".into() } else { "them".into() }),
                text, timestamp, is_sender,
            }
        }).collect();

        (messages, thread_title)
    }
}

impl Drop for InstagramClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Note validation (now done in bridge, but test parse helpers) ────

    #[test]
    fn parse_threads_empty_inbox() {
        let body = json!({"inbox": {"threads": []}});
        let threads = InstagramClient::parse_threads_response(&body);
        assert!(threads.is_empty());
    }

    #[test]
    fn parse_threads_missing_inbox() {
        let body = json!({});
        let threads = InstagramClient::parse_threads_response(&body);
        assert!(threads.is_empty());
    }

    #[test]
    fn parse_threads_basic() {
        let body = json!({
            "inbox": {
                "threads": [{
                    "thread_id": "thread_001",
                    "thread_title": "Alice",
                    "users": [{"username": "alice"}],
                    "items": [{"text": "hey there"}]
                }]
            }
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, "thread_001");
        assert_eq!(threads[0].thread_title, "Alice");
        assert_eq!(threads[0].usernames, vec!["alice"]);
        assert_eq!(threads[0].last_message, "hey there");
    }

    #[test]
    fn parse_threads_empty_title_falls_back_to_usernames() {
        let body = json!({
            "inbox": {"threads": [{"thread_id": "t1", "thread_title": "", "users": [{"username": "bob"}, {"username": "carol"}], "items": [{"text": "hello"}]}]}
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads[0].thread_title, "bob, carol");
    }

    #[test]
    fn parse_threads_media_item_falls_back_to_type() {
        let body = json!({
            "inbox": {"threads": [{"thread_id": "t1", "thread_title": "Alice", "users": [{"username": "alice"}], "items": [{"item_type": "reel_share"}]}]}
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads[0].last_message, "reel_share");
    }

    #[test]
    fn parse_threads_no_items_shows_media() {
        let body = json!({
            "inbox": {"threads": [{"thread_id": "t1", "thread_title": "Alice", "users": [], "items": []}]}
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads[0].last_message, "[media]");
    }

    #[test]
    fn parse_threads_multiple() {
        let body = json!({
            "inbox": {"threads": [
                {"thread_id": "t1", "thread_title": "Alice", "users": [{"username": "alice"}], "items": [{"text": "hi"}]},
                {"thread_id": "t2", "thread_title": "Bob", "users": [{"username": "bob"}], "items": [{"text": "yo"}]}
            ]}
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads.len(), 2);
    }

    #[test]
    fn parse_messages_empty_thread() {
        let body = json!({"thread": {"thread_title": "Alice", "items": [], "users": []}});
        let (msgs, title) = InstagramClient::parse_messages_response(&body, "999");
        assert!(msgs.is_empty());
        assert_eq!(title, "Alice");
    }

    #[test]
    fn parse_messages_identifies_sender() {
        let body = json!({
            "thread": {
                "thread_title": "Alice",
                "users": [{"pk": 111, "username": "alice"}],
                "items": [
                    {"user_id": 999, "text": "from me"},
                    {"user_id": 111, "text": "from alice"}
                ]
            }
        });
        let (msgs, _) = InstagramClient::parse_messages_response(&body, "999");
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].is_sender);
        assert!(!msgs[1].is_sender);
        assert_eq!(msgs[1].user_id, "alice");
    }

    #[test]
    fn parse_messages_media_fallback() {
        let body = json!({
            "thread": {"thread_title": "Test", "users": [], "items": [
                {"user_id": 1, "item_type": "reel_share"},
                {"user_id": 1},
            ]}
        });
        let (msgs, _) = InstagramClient::parse_messages_response(&body, "999");
        assert_eq!(msgs[0].text, "[reel_share]");
        assert_eq!(msgs[1].text, "[media]");
    }

    #[test]
    fn parse_messages_unknown_user_shows_them() {
        let body = json!({"thread": {"thread_title": "Test", "users": [], "items": [{"user_id": 777, "text": "mystery"}]}});
        let (msgs, _) = InstagramClient::parse_messages_response(&body, "999");
        assert_eq!(msgs[0].user_id, "them");
    }

    #[test]
    fn parse_messages_missing_thread() {
        let body = json!({});
        let (msgs, title) = InstagramClient::parse_messages_response(&body, "999");
        assert!(msgs.is_empty());
        assert_eq!(title, "");
    }
}
