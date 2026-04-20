from __future__ import annotations

from textual import work
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import VerticalScroll
from textual.screen import Screen
from textual.widgets import Footer, Input, Label, Static

MESSAGES_PER_PAGE = 20


def _msg_text(msg) -> str:
    if msg.text:
        return msg.text
    item_type = msg.item_type or ""
    if item_type == "reel_share":
        return "[reel]"
    if item_type == "media_share":
        return "[post]"
    if item_type == "story_share":
        return "[story]"
    if item_type == "clip":
        return "[clip]"
    if item_type == "animated_media":
        return "[gif]"
    if item_type in ("media", "visual_media"):
        return "[photo/video]"
    if item_type == "xma_share":
        return "[shared media]"
    if item_type == "link":
        if msg.link and msg.link.text:
            return msg.link.text
        return "[link]"
    return f"[{item_type or 'media'}]"


class DMThreadScreen(Screen):
    BINDINGS = [
        Binding("escape", "go_back", "Back", show=True),
        Binding("r", "refresh", "Refresh", show=True, priority=True),
        Binding("o", "load_older", "Older", show=True, priority=True),
    ]

    CSS = """
    #messages-scroll {
        height: 1fr;
        padding: 0 1;
    }
    .msg-sent {
        text-align: right;
        color: $accent;
        margin: 0 0 0 10;
    }
    .msg-received {
        text-align: left;
        margin: 0 10 0 0;
    }
    .msg-sender {
        text-style: bold;
        height: 1;
    }
    .load-more {
        text-align: center;
        color: $text-muted;
    }
    #reply-input {
        dock: bottom;
        margin: 0 1;
    }
    #status-line {
        dock: bottom;
        height: 1;
        padding: 0 1;
        background: $accent;
        color: $text;
    }
    """

    def __init__(self, thread) -> None:
        super().__init__()
        self._thread = thread
        self._thread_id = thread.id
        users = thread.users
        self._title = ", ".join(u.username for u in users) if users else "unknown"
        self._loaded_amount = MESSAGES_PER_PAGE

    def compose(self) -> ComposeResult:
        yield VerticalScroll(id="messages-scroll")
        yield Static(f"{self._title}", id="status-line")
        yield Input(placeholder="reply...", id="reply-input")
        yield Footer()

    def on_mount(self) -> None:
        # Show thread's cached messages immediately
        if self._thread.messages:
            self._render_messages(self._thread.messages)
            self._set_status(
                f"{self._title}  {len(self._thread.messages)} msgs (cached)  [R] load fresh | [O] older | Esc back"
            )
        else:
            self._load_messages()

    def action_go_back(self) -> None:
        self.app.pop_screen()

    def action_refresh(self) -> None:
        self._loaded_amount = MESSAGES_PER_PAGE
        self._load_messages()

    def action_load_older(self) -> None:
        self._loaded_amount += MESSAGES_PER_PAGE
        self._load_messages(keep_scroll_top=True)

    def on_input_submitted(self, event: Input.Submitted) -> None:
        text = event.value.strip()
        if text:
            event.input.value = ""
            self._send_reply(text)

    @work(thread=True)
    def _load_messages(self, keep_scroll_top: bool = False) -> None:
        self._set_status(f"{self._title}  loading...")
        try:
            client = self.app.ig_client  # type: ignore[attr-defined]
            messages = client.get_thread_messages(self._thread_id, amount=self._loaded_amount)
            self.app.call_from_thread(self._render_messages, messages, keep_scroll_top)
            self._set_status(
                f"{self._title}  {len(messages)} msgs  [O] older | [R] refresh | Esc back"
            )
        except Exception as e:
            # If API call fails, fall back to thread's cached messages
            if self._thread.messages:
                self.app.call_from_thread(self._render_messages, self._thread.messages)
                self._set_status(f"{self._title}  showing cached msgs  error: {e}")
            else:
                self._set_status(f"error: {e}")

    def _render_messages(self, messages: list, keep_scroll_top: bool = False) -> None:
        scroll = self.query_one("#messages-scroll", VerticalScroll)
        scroll.remove_children()

        if len(messages) >= self._loaded_amount:
            scroll.mount(Label("— [O] load older —", classes="load-more"))

        for msg in reversed(messages):
            is_me = bool(msg.is_sent_by_viewer)
            text = _msg_text(msg)
            sender_name = "you" if is_me else self._get_username(str(msg.user_id))
            css_class = "msg-sent" if is_me else "msg-received"
            scroll.mount(Label(f"{sender_name}:", classes=f"msg-sender {css_class}"))
            scroll.mount(Label(text, classes=css_class))

        if keep_scroll_top:
            scroll.scroll_home(animate=False)
        else:
            scroll.scroll_end(animate=False)

    def _get_username(self, user_id: str) -> str:
        for u in self._thread.users:
            if str(u.pk) == user_id:
                return u.username
        return "them"

    @work(thread=True)
    def _send_reply(self, text: str) -> None:
        self._set_status(f"{self._title}  sending...")
        try:
            client = self.app.ig_client  # type: ignore[attr-defined]
            client.send_dm(self._thread_id, text)
            self._set_status(f"{self._title}  sent!")
            self._loaded_amount = MESSAGES_PER_PAGE
            self._load_messages()
        except Exception as e:
            self._set_status(f"error: {e}")

    def _set_status(self, msg: str) -> None:
        def _update() -> None:
            self.query_one("#status-line", Static).update(msg)
        try:
            self.app.call_from_thread(_update)
        except RuntimeError:
            _update()
