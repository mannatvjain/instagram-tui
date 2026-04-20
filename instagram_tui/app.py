from __future__ import annotations

from textual import work
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
        dock: bottom;
        height: 1;
        background: $accent;
        color: $text;
        padding: 0 1;
    }
    #welcome {
        margin: 2 2;
    }
    #keyhints {
        margin: 0 2;
        color: $text-muted;
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
        if self._config.has_session():
            self._try_restore()
        else:
            self._show_login()

    @work(thread=True)
    def _try_restore(self) -> None:
        self._set_status("restoring session...")
        ok = self.ig_client.restore_session()
        if ok:
            username = self.ig_client.get_username()
            self._set_status(f"@{username}")
        else:
            self.app.call_from_thread(self._show_login)

    def _show_login(self) -> None:
        from instagram_tui.screens.login import LoginScreen
        self.push_screen(LoginScreen())

    def on_login_success(self) -> None:
        self.pop_screen()
        username = self.ig_client.get_username()
        self._set_status(f"@{username}")

    def action_open_notes(self) -> None:
        if not self.ig_client.logged_in:
            self._set_status("not logged in")
            return
        from instagram_tui.screens.notes import NotesScreen
        self.push_screen(NotesScreen())

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
