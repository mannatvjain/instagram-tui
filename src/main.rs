mod api;
mod config;
mod tui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "inote", about = "Write-only Instagram TUI")]
struct Cli {
    /// Clear saved session and exit
    #[arg(long)]
    logout: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = config::ConfigStore::new()?;

    if cli.logout {
        store.clear()?;
        println!("Logged out.");
        return Ok(());
    }

    let api = api::InstagramClient::new()?;

    // Get username from saved session
    let session = store.load_session();
    let username = session
        .and_then(|s| s.username)
        .unwrap_or_else(|| "unknown".to_string());

    let mut terminal = ratatui::init();
    let result = tui::run(&mut terminal, api, store, username);
    ratatui::restore();

    result
}
