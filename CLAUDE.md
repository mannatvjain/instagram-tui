# Instagram TUI

## Stack
- **Language**: Python 3.12 (Miniconda at `/opt/homebrew/Caskroom/miniconda`)
- **TUI Framework**: Textual (`textual>=0.80`)
- **Instagram API**: instagrapi (`instagrapi>=2.1,<3`)
- **Packaging**: hatchling + pyproject.toml

## Architecture

### Write-only design
This is a write-only Instagram client — no feed browsing. Two capabilities:
1. **Notes** — compose and publish 60-char Instagram notes from the terminal
2. **DMs** — read threads, view messages, send replies

### Files
- `instagram_tui/__main__.py` — CLI entry point, `--logout` flag
- `instagram_tui/app.py` — Main `InstagramTUI` Textual app: session restore, screen routing
- `instagram_tui/client.py` — `InstagramClient` wrapper around instagrapi: login, notes, DMs
- `instagram_tui/config.py` — `ConfigStore`: credentials + session persistence at `~/.config/instagram-tui/`
- `instagram_tui/screens/login.py` — Login screen with username/password/2FA inputs
- `instagram_tui/screens/notes.py` — Notes composer with char counter (60 limit)
- `instagram_tui/screens/dm_list.py` — DM thread list with preview
- `instagram_tui/screens/dm_thread.py` — Thread view with message history + reply input
- `instagram_tui/styles/app.tcss` — Base Textual stylesheet

### State files
- `~/.config/instagram-tui/config.json` — Saved username
- `~/.config/instagram-tui/session.json` — instagrapi session (cookies, auth tokens)

## Conventions
- Screens use `@work(thread=True)` for all API calls to avoid blocking the UI
- Each screen manages its own `#status-line` Static widget for feedback
- Session is persisted after every API call that mutates state (login, note, DM)
- Config files are `chmod 0o600`

## Dev commands
```bash
pip install -e .          # Install in dev mode
instagram-tui             # Run the TUI
instagram-tui --logout    # Clear saved session
inote                     # Alias for instagram-tui
```
