use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::SessionConfig;

const IG_BASE: &str = "https://i.instagram.com/api/v1";
const USER_AGENT: &str = "Instagram 317.0.0.34.109 Android (30/11; 420dpi; 1080x2220; Google; Pixel 4; flame; qcom; en_US; 556895546)";

pub struct InstagramClient {
    http: Client,
}

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

impl InstagramClient {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            .cookie_store(true)
            .user_agent(USER_AGENT)
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { http })
    }

    fn headers(&self, session: &SessionConfig) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("X-IG-App-ID", HeaderValue::from_static("936619743392459"));
        h.insert("X-IG-Device-ID", HeaderValue::from_static("00000000-0000-0000-0000-000000000000"));
        h.insert("X-IG-Android-ID", HeaderValue::from_static("android-0000000000000000"));
        if let Some(ref csrf) = session.csrf_token {
            if let Ok(v) = HeaderValue::from_str(csrf) {
                h.insert("X-CSRFToken", v);
            }
        }
        if let Some(ref cookies) = session.cookies {
            if let Ok(v) = HeaderValue::from_str(cookies) {
                h.insert("Cookie", v);
            }
        }
        h
    }

    pub fn login(&self, username: &str, password: &str) -> Result<SessionConfig> {
        // Step 1: Get CSRF token
        let resp = self
            .http
            .get("https://www.instagram.com/api/v1/web/accounts/login/ajax/")
            .header("User-Agent", USER_AGENT)
            .header("X-IG-App-ID", "936619743392459")
            .send()
            .context("failed to fetch CSRF")?;

        let csrf = resp
            .cookies()
            .find(|c| c.name() == "csrftoken")
            .map(|c| c.value().to_string())
            .unwrap_or_default();

        // Step 2: Login
        let resp = self
            .http
            .post("https://www.instagram.com/api/v1/web/accounts/login/ajax/")
            .header("User-Agent", USER_AGENT)
            .header("X-CSRFToken", &csrf)
            .header("X-IG-App-ID", "936619743392459")
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", "https://www.instagram.com/")
            .form(&[
                ("username", username),
                ("enc_password", &format!("#PWD_INSTAGRAM_BROWSER:0:{}:{}", chrono::Utc::now().timestamp(), password)),
                ("queryParams", "{}"),
                ("optIntoOneTap", "false"),
            ])
            .send()
            .context("login request failed")?;

        let new_csrf = resp
            .cookies()
            .find(|c| c.name() == "csrftoken")
            .map(|c| c.value().to_string())
            .unwrap_or(csrf.clone());

        let session_id = resp
            .cookies()
            .find(|c| c.name() == "sessionid")
            .map(|c| c.value().to_string());

        let cookies: Vec<String> = resp
            .cookies()
            .map(|c| format!("{}={}", c.name(), c.value()))
            .collect();

        let body: Value = resp.json().context("failed to parse login response")?;

        if body.get("authenticated") == Some(&Value::Bool(true)) {
            let user_id = body
                .get("userId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            Ok(SessionConfig {
                username: Some(username.to_string()),
                session_id,
                csrf_token: Some(new_csrf),
                user_id: Some(user_id),
                cookies: Some(cookies.join("; ")),
            })
        } else if body.get("two_factor_required") == Some(&Value::Bool(true)) {
            bail!("2FA required — not yet implemented in Rust TUI")
        } else {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("login failed");
            bail!("login failed: {}", msg)
        }
    }

    #[allow(dead_code)]
    pub fn validate_session(&self, session: &SessionConfig) -> bool {
        let resp = self
            .http
            .get(format!("{}/accounts/current_user/?edit=true", IG_BASE))
            .headers(self.headers(session))
            .send();

        match resp {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    pub fn create_note(&self, session: &SessionConfig, text: &str) -> Result<String> {
        if text.is_empty() {
            bail!("note cannot be empty");
        }
        if text.len() > 60 {
            bail!("note too long: {}/60 characters", text.len());
        }

        let resp = self
            .http
            .post(format!("{}/notes/create_note/", IG_BASE))
            .headers(self.headers(session))
            .form(&[
                ("text", text),
                ("audience", "0"),
            ])
            .send()
            .context("create note request failed")?;

        let body: Value = resp.json().context("failed to parse note response")?;

        if body.get("status").and_then(|v| v.as_str()) == Some("ok") {
            let note_id = body
                .pointer("/note/id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            Ok(note_id)
        } else {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("failed to create note");
            bail!("{}", msg)
        }
    }

    pub fn get_direct_threads(&self, session: &SessionConfig, limit: usize) -> Result<Vec<DirectThread>> {
        let resp = self
            .http
            .get(format!("{}/direct_v2/inbox/?persistentBadging=true&limit={}", IG_BASE, limit))
            .headers(self.headers(session))
            .send()
            .context("failed to fetch DM inbox")?;

        let body: Value = resp.json().context("failed to parse inbox response")?;
        Ok(Self::parse_threads_response(&body))
    }

    pub fn get_thread_messages(
        &self,
        session: &SessionConfig,
        thread_id: &str,
        limit: usize,
    ) -> Result<(Vec<DirectMessage>, String)> {
        let resp = self
            .http
            .get(format!(
                "{}/direct_v2/threads/{}/?limit={}",
                IG_BASE, thread_id, limit
            ))
            .headers(self.headers(session))
            .send()
            .context("failed to fetch thread messages")?;

        let body: Value = resp.json().context("failed to parse thread response")?;
        let my_user_id = session.user_id.as_deref().unwrap_or("");
        Ok(Self::parse_messages_response(&body, my_user_id))
    }

    pub fn parse_threads_response(body: &Value) -> Vec<DirectThread> {
        let threads = body
            .pointer("/inbox/threads")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut result = Vec::new();
        for t in threads {
            let thread_id = t
                .get("thread_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let thread_title = t
                .get("thread_title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let usernames: Vec<String> = t
                .get("users")
                .and_then(|v| v.as_array())
                .map(|users| {
                    users
                        .iter()
                        .filter_map(|u| u.get("username").and_then(|v| v.as_str()))
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();

            let last_message = t
                .get("items")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
                .and_then(|item| {
                    item.get("text")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("item_type").and_then(|v| v.as_str()))
                })
                .unwrap_or("[media]")
                .to_string();

            let title = if thread_title.is_empty() {
                usernames.join(", ")
            } else {
                thread_title
            };

            result.push(DirectThread {
                thread_id,
                thread_title: title,
                usernames,
                last_message,
            });
        }

        result
    }

    pub fn parse_messages_response(body: &Value, my_user_id: &str) -> (Vec<DirectMessage>, String) {
        let thread_title = body
            .pointer("/thread/thread_title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let items = body
            .pointer("/thread/items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let users = body
            .pointer("/thread/users")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let user_map: std::collections::HashMap<String, String> = users
            .iter()
            .filter_map(|u| {
                let pk = u.get("pk")?.to_string().trim_matches('"').to_string();
                let name = u.get("username")?.as_str()?.to_string();
                Some((pk, name))
            })
            .collect();

        let mut messages = Vec::new();
        for item in &items {
            let user_id = item
                .get("user_id")
                .map(|v| v.to_string().trim_matches('"').to_string())
                .unwrap_or_default();

            let text = item
                .get("text")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    let item_type = item
                        .get("item_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("media");
                    format!("[{}]", item_type)
                });

            let timestamp = item
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let is_sender = user_id == my_user_id;

            messages.push(DirectMessage {
                user_id: user_map
                    .get(&user_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        if is_sender {
                            "you".to_string()
                        } else {
                            "them".to_string()
                        }
                    }),
                text,
                timestamp,
                is_sender,
            });
        }

        (messages, thread_title)
    }

    pub fn send_dm(&self, session: &SessionConfig, thread_id: &str, text: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/direct_v2/threads/{}/items/text/", IG_BASE, thread_id))
            .headers(self.headers(session))
            .form(&[
                ("text", text),
                ("action", "send_item"),
            ])
            .send()
            .context("send DM request failed")?;

        let body: Value = resp.json().context("failed to parse send response")?;

        if body.get("status").and_then(|v| v.as_str()) == Some("ok") {
            Ok(())
        } else {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("failed to send message");
            bail!("{}", msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_session() -> SessionConfig {
        SessionConfig {
            username: Some("testuser".to_string()),
            session_id: Some("sid123".to_string()),
            csrf_token: Some("csrf456".to_string()),
            user_id: Some("12345".to_string()),
            cookies: Some("sessionid=sid123; csrftoken=csrf456".to_string()),
        }
    }

    // ── Header tests ────────────────────────────────────────────────────

    #[test]
    fn headers_include_csrf_token() {
        let client = InstagramClient::new().unwrap();
        let session = test_session();
        let headers = client.headers(&session);
        assert_eq!(headers.get("X-CSRFToken").unwrap(), "csrf456");
    }

    #[test]
    fn headers_include_cookies() {
        let client = InstagramClient::new().unwrap();
        let session = test_session();
        let headers = client.headers(&session);
        assert!(headers.get("Cookie").unwrap().to_str().unwrap().contains("sessionid=sid123"));
    }

    #[test]
    fn headers_without_csrf_omit_it() {
        let client = InstagramClient::new().unwrap();
        let session = SessionConfig::default();
        let headers = client.headers(&session);
        assert!(headers.get("X-CSRFToken").is_none());
    }

    #[test]
    fn headers_always_include_app_id() {
        let client = InstagramClient::new().unwrap();
        let session = SessionConfig::default();
        let headers = client.headers(&session);
        assert_eq!(headers.get("X-IG-App-ID").unwrap(), "936619743392459");
    }

    // ── Note validation tests ───────────────────────────────────────────

    #[test]
    fn create_note_rejects_empty() {
        let client = InstagramClient::new().unwrap();
        let session = test_session();
        let result = client.create_note(&session, "");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn create_note_rejects_too_long() {
        let client = InstagramClient::new().unwrap();
        let session = test_session();
        let long_text = "a".repeat(61);
        let result = client.create_note(&session, &long_text);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("61/60"));
    }

    #[test]
    fn create_note_accepts_60_chars() {
        let client = InstagramClient::new().unwrap();
        let session = test_session();
        let text = "a".repeat(60);
        // This will fail with a network error (no real server), but it
        // should NOT fail validation
        let result = client.create_note(&session, &text);
        // The error should be a network error, not a validation error
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(!msg.contains("empty"));
            assert!(!msg.contains("too long"));
        }
    }

    // ── Thread parsing tests ────────────────────────────────────────────

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
                "threads": [
                    {
                        "thread_id": "thread_001",
                        "thread_title": "Alice",
                        "users": [{"username": "alice"}],
                        "items": [{"text": "hey there"}]
                    }
                ]
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
            "inbox": {
                "threads": [
                    {
                        "thread_id": "t1",
                        "thread_title": "",
                        "users": [{"username": "bob"}, {"username": "carol"}],
                        "items": [{"text": "hello"}]
                    }
                ]
            }
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads[0].thread_title, "bob, carol");
    }

    #[test]
    fn parse_threads_media_item_falls_back_to_type() {
        let body = json!({
            "inbox": {
                "threads": [
                    {
                        "thread_id": "t1",
                        "thread_title": "Alice",
                        "users": [{"username": "alice"}],
                        "items": [{"item_type": "reel_share"}]
                    }
                ]
            }
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads[0].last_message, "reel_share");
    }

    #[test]
    fn parse_threads_no_items_shows_media() {
        let body = json!({
            "inbox": {
                "threads": [
                    {
                        "thread_id": "t1",
                        "thread_title": "Alice",
                        "users": [],
                        "items": []
                    }
                ]
            }
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads[0].last_message, "[media]");
    }

    #[test]
    fn parse_threads_multiple() {
        let body = json!({
            "inbox": {
                "threads": [
                    {
                        "thread_id": "t1",
                        "thread_title": "Alice",
                        "users": [{"username": "alice"}],
                        "items": [{"text": "hi"}]
                    },
                    {
                        "thread_id": "t2",
                        "thread_title": "Bob",
                        "users": [{"username": "bob"}],
                        "items": [{"text": "yo"}]
                    }
                ]
            }
        });
        let threads = InstagramClient::parse_threads_response(&body);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].thread_id, "t1");
        assert_eq!(threads[1].thread_id, "t2");
    }

    // ── Message parsing tests ───────────────────────────────────────────

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
        assert_eq!(msgs[0].text, "from me");
        assert!(!msgs[1].is_sender);
        assert_eq!(msgs[1].text, "from alice");
        assert_eq!(msgs[1].user_id, "alice");
    }

    #[test]
    fn parse_messages_media_fallback() {
        let body = json!({
            "thread": {
                "thread_title": "Test",
                "users": [],
                "items": [
                    {"user_id": 1, "item_type": "reel_share"},
                    {"user_id": 1, "item_type": "media_share"},
                    {"user_id": 1}
                ]
            }
        });
        let (msgs, _) = InstagramClient::parse_messages_response(&body, "999");
        assert_eq!(msgs[0].text, "[reel_share]");
        assert_eq!(msgs[1].text, "[media_share]");
        assert_eq!(msgs[2].text, "[media]");
    }

    #[test]
    fn parse_messages_unknown_user_shows_them() {
        let body = json!({
            "thread": {
                "thread_title": "Test",
                "users": [],
                "items": [{"user_id": 777, "text": "mystery"}]
            }
        });
        let (msgs, _) = InstagramClient::parse_messages_response(&body, "999");
        assert_eq!(msgs[0].user_id, "them");
        assert!(!msgs[0].is_sender);
    }

    #[test]
    fn parse_messages_missing_thread() {
        let body = json!({});
        let (msgs, title) = InstagramClient::parse_messages_response(&body, "999");
        assert!(msgs.is_empty());
        assert_eq!(title, "");
    }
}
