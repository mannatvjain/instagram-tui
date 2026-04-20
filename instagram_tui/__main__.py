import argparse

from instagram_tui.app import InstagramTUI
from instagram_tui.config import ConfigStore


def main() -> None:
    parser = argparse.ArgumentParser(description="Instagram TUI")
    parser.add_argument("--logout", action="store_true", help="Clear saved session and exit")
    args = parser.parse_args()

    if args.logout:
        ConfigStore.clear()
        print("Logged out.")
        return

    app = InstagramTUI()
    app.run()


if __name__ == "__main__":
    main()
