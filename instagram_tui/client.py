from __future__ import annotations

from instagrapi import Client
from instagrapi.exceptions import LoginRequired

from instagram_tui.config import ConfigStore


class InstagramClient:
    def __init__(self, config: ConfigStore) -> None:
        self.config = config
        self.cl = Client()
        self._logged_in = False

    def restore_session(self) -> bool:
        session = self.config.get_session()
        if session is None:
            return False
        try:
            self.cl.set_settings(session)
            self.cl.login_by_sessionid(session.get("authorization_data", {}).get("sessionid", ""))
            # Validate session with a lightweight call
            self.cl.get_timeline_feed()
            self._logged_in = True
            return True
        except Exception:
            return False

    def login(self, username: str, password: str, totp_code: str = "") -> None:
        if totp_code:
            self.cl.totp_code = totp_code
        self.cl.login(username, password)
        self.config.save_credentials(username)
        self.config.save_session(self.cl.get_settings())
        self._logged_in = True

    @property
    def logged_in(self) -> bool:
        return self._logged_in

    def get_username(self) -> str:
        return self.cl.username or self.config.username or "unknown"

    def create_note(self, text: str, audience: int = 0) -> str:
        text = text.strip()
        if not text:
            raise ValueError("Note cannot be empty")
        if len(text) > 60:
            raise ValueError(f"Note too long: {len(text)}/60 characters")
        result = self.cl.create_note(text, audience)
        self.config.save_session(self.cl.get_settings())
        return str(result.id)

    def get_direct_threads(self, amount: int = 20) -> list:
        try:
            threads = self.cl.direct_threads(amount=amount)
        except LoginRequired:
            raise
        return threads

    def get_thread_messages(self, thread_id: str, amount: int = 20) -> list:
        return self.cl.direct_messages(thread_id, amount=amount)

    def send_dm(self, thread_id: str, text: str) -> None:
        self.cl.direct_answer(thread_id, text)
        self.config.save_session(self.cl.get_settings())
