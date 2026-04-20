use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionConfig {
    pub username: Option<String>,
    pub session_id: Option<String>,
    pub csrf_token: Option<String>,
    pub user_id: Option<String>,
    pub cookies: Option<String>,
}

pub struct ConfigStore {
    config_dir: PathBuf,
}

impl ConfigStore {
    pub fn new() -> Result<Self> {
        let dirs = ProjectDirs::from("com", "mannat", "instagram-tui")
            .context("failed to determine config directory")?;
        let config_dir = dirs.config_dir().to_path_buf();
        fs::create_dir_all(&config_dir)
            .with_context(|| format!("failed to create config dir: {}", config_dir.display()))?;
        Ok(Self { config_dir })
    }

    pub fn from_path(config_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&config_dir)
            .with_context(|| format!("failed to create config dir: {}", config_dir.display()))?;
        Ok(Self { config_dir })
    }

    fn session_path(&self) -> PathBuf {
        self.config_dir.join("session.json")
    }

    pub fn load_session(&self) -> Option<SessionConfig> {
        let path = self.session_path();
        if !path.exists() {
            return None;
        }
        let data = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn save_session(&self, session: &SessionConfig) -> Result<()> {
        let path = self.session_path();
        let data = serde_json::to_string_pretty(session)?;
        fs::write(&path, &data).context("failed to write session")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let path = self.session_path();
        if path.exists() {
            fs::remove_file(&path).context("failed to remove session")?;
        }
        let cache = self.cache_path();
        if cache.exists() {
            fs::remove_file(&cache).context("failed to remove cache")?;
        }
        Ok(())
    }

    fn cache_path(&self) -> PathBuf {
        self.config_dir.join("dm_cache.json")
    }

    pub fn load_cache(&self) -> Option<DmCache> {
        let path = self.cache_path();
        if !path.exists() {
            return None;
        }
        let data = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn save_cache(&self, cache: &DmCache) -> Result<()> {
        let data = serde_json::to_string(cache)?;
        fs::write(self.cache_path(), &data).context("failed to write cache")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DmCache {
    pub threads: Vec<crate::api::DirectThread>,
    pub messages: std::collections::HashMap<String, Vec<crate::api::DirectMessage>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> (ConfigStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::from_path(dir.path().to_path_buf()).unwrap();
        (store, dir)
    }

    #[test]
    fn load_returns_none_when_no_session() {
        let (store, _dir) = tmp_store();
        assert!(store.load_session().is_none());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let (store, _dir) = tmp_store();
        let session = SessionConfig {
            username: Some("testuser".to_string()),
            session_id: Some("abc123".to_string()),
            csrf_token: Some("token456".to_string()),
            user_id: Some("999".to_string()),
            cookies: Some("sessionid=abc123; csrftoken=token456".to_string()),
        };
        store.save_session(&session).unwrap();

        let loaded = store.load_session().expect("session should exist");
        assert_eq!(loaded.username.as_deref(), Some("testuser"));
        assert_eq!(loaded.session_id.as_deref(), Some("abc123"));
        assert_eq!(loaded.csrf_token.as_deref(), Some("token456"));
        assert_eq!(loaded.user_id.as_deref(), Some("999"));
        assert!(loaded.cookies.as_deref().unwrap().contains("sessionid=abc123"));
    }

    #[test]
    fn clear_removes_session() {
        let (store, _dir) = tmp_store();
        let session = SessionConfig {
            username: Some("testuser".to_string()),
            ..Default::default()
        };
        store.save_session(&session).unwrap();
        assert!(store.load_session().is_some());

        store.clear().unwrap();
        assert!(store.load_session().is_none());
    }

    #[test]
    fn clear_is_idempotent() {
        let (store, _dir) = tmp_store();
        store.clear().unwrap(); // no file to remove
        store.clear().unwrap(); // still fine
    }

    #[test]
    fn save_overwrites_existing() {
        let (store, _dir) = tmp_store();
        let s1 = SessionConfig {
            username: Some("first".to_string()),
            ..Default::default()
        };
        store.save_session(&s1).unwrap();

        let s2 = SessionConfig {
            username: Some("second".to_string()),
            ..Default::default()
        };
        store.save_session(&s2).unwrap();

        let loaded = store.load_session().unwrap();
        assert_eq!(loaded.username.as_deref(), Some("second"));
    }

    #[cfg(unix)]
    #[test]
    fn session_file_has_600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (store, _dir) = tmp_store();
        let session = SessionConfig {
            username: Some("test".to_string()),
            ..Default::default()
        };
        store.save_session(&session).unwrap();

        let perms = fs::metadata(store.session_path()).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[test]
    fn default_session_config_has_all_none() {
        let s = SessionConfig::default();
        assert!(s.username.is_none());
        assert!(s.session_id.is_none());
        assert!(s.csrf_token.is_none());
        assert!(s.user_id.is_none());
        assert!(s.cookies.is_none());
    }

    #[test]
    fn corrupt_json_returns_none() {
        let (store, _dir) = tmp_store();
        fs::write(store.session_path(), "not valid json!!!").unwrap();
        assert!(store.load_session().is_none());
    }
}
