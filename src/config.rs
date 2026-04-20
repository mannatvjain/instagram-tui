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
        Ok(())
    }
}
