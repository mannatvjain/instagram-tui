from __future__ import annotations

import threading

from instagrapi.exceptions import LoginRequired
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.widgets import Footer, Static

from instagram_tui.client import InstagramClient
from instagram_tui.config import ConfigStore


class InstagramTUI(App):
    TITLE = "inote"
    BINDINGS = [
        Binding("n", "open_notes", "Notes", show=True),
        Binding("d", "open_dms", "DMs", show=True),
        Binding("q", "quit", "Quit", show=True),
    ]

    CSS = """
    Screen {
        background: $surface;
    }
    #status-line {
        height: 1;
        padding: 0 2;
        color: $text-muted;
    }
    #welcome {
        margin: 2 2;
    }
    #keyhints {
        margin: 0 2;
        color: $text-muted;
    }
    Footer {
        height: 3;
        padding: 1 1;
    }
    """

    def __init__(self) -> None:
        super().__init__()
        self._config = ConfigStore()
        self.ig_client = InstagramClient(self._config)

    def compose(self) -> ComposeResult:
        yield Static("inote", id="welcome")
        yield Static("[N] notes    [D] dms    [Q] quit", id="keyhints")
        yield Static("", id="status-line")
        yield Footer()

    def on_mount(self) -> None:
        from instagram_tui.screens.notes import NotesScreen
        self.install_screen(NotesScreen(), name="notes")

        if self._config.has_session():
            self._try_restore()
        else:
            self._show_login()

    def _try_restore(self) -> None:
        ok = self.ig_client.restore_session()
        if ok:
            username = self.ig_client.get_username()
            self._set_status(f"@{username}")
            self._prefetch_dms()
        else:
            self._show_login()

    def _prefetch_dms(self) -> None:
        def _fetch() -> None:
            try:
                self._cached_threads = self.ig_client.get_direct_threads(amount=20)
            except LoginRequired:
                self.call_from_thread(self.handle_login_required)
            except Exception:
                pass
        self._cached_threads = None
        threading.Thread(target=_fetch, daemon=True).start()

    def _show_login(self) -> None:
        from instagram_tui.screens.login import LoginScreen
        self.push_screen(LoginScreen())

    def on_login_success(self) -> None:
        self.pop_screen()
        username = self.ig_client.get_username()
        self._set_status(f"@{username}")
        self._prefetch_dms()

    def handle_login_required(self) -> None:
        self.ig_client._logged_in = False
        self._set_status("session expired")
        self._show_login()

    def action_open_notes(self) -> None:
        if not self.ig_client.logged_in:
            self._set_status("not logged in")
            return
        self.push_screen("notes")

    def action_open_dms(self) -> None:
        if not self.ig_client.logged_in:
            self._set_status("not logged in")
            return
        from instagram_tui.screens.dm_list import DMListScreen
        self.push_screen(DMListScreen())

    def _set_status(self, msg: str) -> None:
        def _update() -> None:
            self.query_one("#status-line", Static).update(msg)
        try:
            self.app.call_from_thread(_update)
        except RuntimeError:
            _update()
