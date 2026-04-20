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
                        .or_else(|| {
                            item.get("item_type").and_then(|v| v.as_str())
                        })
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

        Ok(result)
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

        // Build username lookup from thread users
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

        Ok((messages, thread_title))
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
