from __future__ import annotations

import json
from pathlib import Path


CONFIG_DIR = Path.home() / ".config" / "instagram-tui"
CONFIG_FILE = CONFIG_DIR / "config.json"
SESSION_FILE = CONFIG_DIR / "session.json"


class ConfigStore:
    def __init__(self) -> None:
        self._username: str | None = None
        self._load()

    def _load(self) -> None:
        if CONFIG_FILE.exists():
            data = json.loads(CONFIG_FILE.read_text())
            self._username = data.get("username")

    @property
    def username(self) -> str | None:
        return self._username

    def has_session(self) -> bool:
        return SESSION_FILE.exists()

    def save_credentials(self, username: str) -> None:
        CONFIG_DIR.mkdir(parents=True, exist_ok=True)
        data = {"username": username}
        CONFIG_FILE.write_text(json.dumps(data, indent=2))
        CONFIG_FILE.chmod(0o600)
        self._username = username

    def save_session(self, settings: dict) -> None:
        CONFIG_DIR.mkdir(parents=True, exist_ok=True)
        SESSION_FILE.write_text(json.dumps(settings, indent=2, default=str))
        SESSION_FILE.chmod(0o600)

    def get_session(self) -> dict | None:
        if not SESSION_FILE.exists():
            return None
        return json.loads(SESSION_FILE.read_text())

    @classmethod
    def clear(cls) -> None:
        for f in (CONFIG_FILE, SESSION_FILE):
            if f.exists():
                f.unlink()
