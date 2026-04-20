from __future__ import annotations

from textual import on, work
from textual.app import ComposeResult
from textual.binding import Binding
from textual.screen import Screen
from textual.widgets import Footer, Static, TextArea

NOTE_CHAR_LIMIT = 60


class NotesScreen(Screen):
    BINDINGS = [
        Binding("escape", "go_back", "Back", show=True),
        Binding("ctrl+s", "publish", "Publish", show=True),
        Binding("ctrl+l", "clear", "Clear", show=True),
    ]

    CSS = """
    #note-textarea {
        height: 1fr;
    }
    #meta-line {
        dock: bottom;
        height: 1;
        padding: 0 1;
    }
    #status-line {
        dock: bottom;
        height: 1;
        padding: 0 1;
        background: $accent;
        color: $text;
    }
    .green { color: green; }
    .yellow { color: yellow; }
    .red { color: red; }
    """

    def compose(self) -> ComposeResult:
        yield TextArea(id="note-textarea")
        yield Static("0 chars  60 left  Ctrl-S publish | Ctrl-L clear | Esc back", id="meta-line", classes="green")
        yield Static("", id="status-line")
        yield Footer()

    def on_mount(self) -> None:
        ta = self.query_one("#note-textarea", TextArea)
        ta.show_line_numbers = False
        ta.focus()

    @on(TextArea.Changed, "#note-textarea")
    def on_text_changed(self, event: TextArea.Changed) -> None:
        count = len(event.text_area.text)
        remaining = NOTE_CHAR_LIMIT - count

        meta = self.query_one("#meta-line", Static)
        meta.remove_class("green", "yellow", "red")

        if remaining < 0:
            meta.add_class("red")
            left_str = f"{abs(remaining)} over"
        elif remaining < 10:
            meta.add_class("yellow")
            left_str = f"{remaining} left"
        else:
            meta.add_class("green")
            left_str = f"{remaining} left"

        meta.update(f"{count} chars  {left_str}  Ctrl-S publish | Ctrl-L clear | Esc back")

    def action_go_back(self) -> None:
        self.app.pop_screen()

    def action_clear(self) -> None:
        self.query_one("#note-textarea", TextArea).text = ""

    def action_publish(self) -> None:
        self._do_publish()

    @work(thread=True)
    def _do_publish(self) -> None:
        text = self.app.call_from_thread(
            lambda: self.query_one("#note-textarea", TextArea).text.strip()
        )
        if not text:
            self._set_status("empty")
            return
        if len(text) > NOTE_CHAR_LIMIT:
            self._set_status(f"too long: {len(text)}/{NOTE_CHAR_LIMIT}")
            return

        self._set_status("publishing...")
        try:
            client = self.app.ig_client  # type: ignore[attr-defined]
            note_id = client.create_note(text)
            self._set_status(f"published (id: {note_id})")
            self.app.call_from_thread(
                lambda: setattr(self.query_one("#note-textarea", TextArea), "text", "")
            )
        except Exception as e:
            self._set_status(f"error: {e}")

    def _set_status(self, msg: str) -> None:
        def _update() -> None:
            self.query_one("#status-line", Static).update(msg)
        try:
            self.app.call_from_thread(_update)
        except RuntimeError:
            _update()
