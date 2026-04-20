from __future__ import annotations

from textual import work
from textual.app import ComposeResult
from textual.binding import Binding
from textual.screen import Screen
from textual.widgets import Footer, ListItem, ListView, Label, Static


class DMListScreen(Screen):
    BINDINGS = [
        Binding("escape", "go_back", "Back", show=True),
        Binding("r", "refresh", "Refresh", show=True),
    ]

    CSS = """
    #dm-listview {
        height: 1fr;
    }
    .thread-item {
        padding: 0 1;
        height: 2;
    }
    .thread-name {
        text-style: bold;
    }
    .thread-preview {
        color: $text-muted;
    }
    #status-line {
        dock: bottom;
        height: 1;
        padding: 0 1;
        background: $accent;
        color: $text;
    }
    """

    def __init__(self) -> None:
        super().__init__()
        self._threads: list = []

    def compose(self) -> ComposeResult:
        yield ListView(id="dm-listview")
        yield Static("", id="status-line")
        yield Footer()

    def on_mount(self) -> None:
        self._load_threads()

    def action_go_back(self) -> None:
        self.app.pop_screen()

    def action_refresh(self) -> None:
        self._load_threads()

    @work(thread=True)
    def _load_threads(self) -> None:
        self._set_status("loading...")
        try:
            client = self.app.ig_client  # type: ignore[attr-defined]
            self._threads = client.get_direct_threads(amount=20)
            self.app.call_from_thread(self._populate_list)
            count = len(self._threads)
            self._set_status(f"{count} conversation{'s' if count != 1 else ''}  [R] refresh | Esc back")
        except Exception as e:
            self._set_status(f"error: {e}")

    def _populate_list(self) -> None:
        lv = self.query_one("#dm-listview", ListView)
        lv.clear()
        for thread in self._threads:
            users = thread.users
            names = ", ".join(u.username for u in users) if users else "unknown"
            last_msg = ""
            if thread.messages:
                msg = thread.messages[0]
                last_msg = (msg.text or "[media]")[:80]

            item = ListItem(
                Label(names, classes="thread-name"),
                Label(last_msg, classes="thread-preview"),
                classes="thread-item",
            )
            lv.append(item)

    def on_list_view_selected(self, event: ListView.Selected) -> None:
        idx = event.list_view.index
        if idx is not None and idx < len(self._threads):
            thread = self._threads[idx]
            from instagram_tui.screens.dm_thread import DMThreadScreen
            self.app.push_screen(DMThreadScreen(thread))

    def _set_status(self, msg: str) -> None:
        def _update() -> None:
            self.query_one("#status-line", Static).update(msg)
        try:
            self.app.call_from_thread(_update)
        except RuntimeError:
            _update()
