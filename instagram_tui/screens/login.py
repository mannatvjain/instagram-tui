from __future__ import annotations

from textual import work
from textual.app import ComposeResult
from textual.binding import Binding
from textual.screen import Screen
from textual.widgets import Footer, Input, Static


class LoginScreen(Screen):
    BINDINGS = [
        Binding("escape", "quit", "Quit", show=True),
    ]

    CSS = """
    #login-title {
        margin: 2 2 1 2;
    }
    .login-input {
        margin: 0 2 1 2;
        width: 60;
    }
    #login-status {
        margin: 1 2;
        height: 1;
    }
    .error { color: red; }
    .info { color: $accent; }
    """

    def compose(self) -> ComposeResult:
        yield Static("login", id="login-title")
        yield Input(placeholder="username", id="username-input", classes="login-input")
        yield Input(placeholder="password", id="password-input", password=True, classes="login-input")
        yield Input(placeholder="2fa code (if needed)", id="totp-input", classes="login-input")
        yield Static("", id="login-status")
        yield Footer()

    def on_mount(self) -> None:
        self.query_one("#username-input", Input).focus()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id == "username-input":
            self.query_one("#password-input", Input).focus()
        elif event.input.id == "password-input":
            self._do_login()
        elif event.input.id == "totp-input":
            self._do_login()

    @work(thread=True)
    def _do_login(self) -> None:
        username = self.app.call_from_thread(
            lambda: self.query_one("#username-input", Input).value.strip()
        )
        password = self.app.call_from_thread(
            lambda: self.query_one("#password-input", Input).value
        )
        totp = self.app.call_from_thread(
            lambda: self.query_one("#totp-input", Input).value.strip()
        )

        if not username or not password:
            self._set_status("username and password required", error=True)
            return

        self._set_status("logging in...", error=False)

        try:
            client = self.app.ig_client  # type: ignore[attr-defined]
            client.login(username, password, totp)
            self._set_status("ok", error=False)
            self.app.call_from_thread(self.app.on_login_success)  # type: ignore[attr-defined]
        except Exception as e:
            err = str(e).lower()
            if "two_factor" in err or "2fa" in err or "challenge" in err:
                self._set_status("enter 2fa code above", error=False)
                self.app.call_from_thread(
                    lambda: self.query_one("#totp-input", Input).focus()
                )
            else:
                self._set_status(f"failed: {str(e)[:80]}", error=True)

    def _set_status(self, msg: str, error: bool) -> None:
        def _update() -> None:
            s = self.query_one("#login-status", Static)
            s.remove_class("error", "info")
            s.add_class("error" if error else "info")
            s.update(msg)
        try:
            self.app.call_from_thread(_update)
        except RuntimeError:
            _update()

    def action_quit(self) -> None:
        self.app.exit()
