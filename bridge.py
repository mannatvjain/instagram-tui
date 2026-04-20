#!/usr/bin/env python3
"""Thin JSON bridge between Rust TUI and instagrapi."""

import json
import sys

from instagrapi import Client

CONFIG_DIR = None
SESSION_FILE = None


def init():
    from pathlib import Path
    global CONFIG_DIR, SESSION_FILE
    CONFIG_DIR = Path.home() / ".config" / "instagram-tui"
    SESSION_FILE = CONFIG_DIR / "session.json"


def load_client() -> Client:
    init()
    cl = Client()
    if SESSION_FILE.exists():
        data = json.loads(SESSION_FILE.read_text())
        cl.set_settings(data)
        sid = data.get("authorization_data", {}).get("sessionid", "")
        cl.login_by_sessionid(sid)
    return cl


def save_session(cl: Client):
    init()
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    SESSION_FILE.write_text(json.dumps(cl.get_settings(), indent=2, default=str))
    SESSION_FILE.chmod(0o600)


def cmd_login(cl: Client, args: dict) -> dict:
    username = args["username"]
    password = args["password"]
    totp = args.get("totp", "")
    if totp:
        cl.totp_code = totp
    cl.login(username, password)
    save_session(cl)
    return {"ok": True, "username": username, "user_id": str(cl.user_id)}


def cmd_threads(cl: Client, args: dict) -> dict:
    amount = args.get("amount", 20)
    threads = cl.direct_threads(amount=amount)
    save_session(cl)
    result = []
    for t in threads:
        users = [u.username for u in t.users]
        last_msg = ""
        if t.messages:
            msg = t.messages[0]
            last_msg = (msg.text or "[media]")[:80]
        result.append({
            "thread_id": t.id,
            "thread_title": ", ".join(users) if users else "unknown",
            "usernames": users,
            "last_message": last_msg,
        })
    return {"ok": True, "threads": result}


def cmd_messages(cl: Client, args: dict) -> dict:
    thread_id = args["thread_id"]
    amount = args.get("amount", 20)
    messages = cl.direct_messages(thread_id, amount=amount)
    save_session(cl)

    my_id = str(cl.user_id)

    # Get thread for user lookup
    thread = cl.direct_thread(thread_id)
    user_map = {str(u.pk): u.username for u in thread.users}
    title = ", ".join(u.username for u in thread.users) if thread.users else "unknown"

    result = []
    for msg in messages:
        uid = str(msg.user_id)
        is_sender = uid == my_id
        text = msg.text or f"[{msg.item_type or 'media'}]"
        result.append({
            "user_id": user_map.get(uid, "you" if is_sender else "them"),
            "text": text,
            "timestamp": str(msg.timestamp) if msg.timestamp else "",
            "is_sender": is_sender,
        })
    return {"ok": True, "messages": result, "title": title}


def cmd_send_dm(cl: Client, args: dict) -> dict:
    thread_id = args["thread_id"]
    text = args["text"]
    cl.direct_answer(int(thread_id), text)
    save_session(cl)
    return {"ok": True}


def cmd_create_note(cl: Client, args: dict) -> dict:
    text = args["text"].strip()
    if not text:
        return {"ok": False, "error": "note cannot be empty"}
    if len(text) > 60:
        return {"ok": False, "error": f"note too long: {len(text)}/60"}
    result = cl.create_note(text, args.get("audience", 0))
    save_session(cl)
    return {"ok": True, "note_id": str(result.id)}


COMMANDS = {
    "login": cmd_login,
    "threads": cmd_threads,
    "messages": cmd_messages,
    "send_dm": cmd_send_dm,
    "create_note": cmd_create_note,
}


def main():
    cl = load_client()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            cmd = req.get("cmd", "")
            args = req.get("args", {})
            if cmd in COMMANDS:
                resp = COMMANDS[cmd](cl, args)
            else:
                resp = {"ok": False, "error": f"unknown command: {cmd}"}
        except Exception as e:
            resp = {"ok": False, "error": str(e)}

        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
