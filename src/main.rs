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

    // Try to restore session
    let session = store.load_session();
    let session = if let Some(ref s) = session {
        if s.session_id.is_some() {
            Some(s.clone())
        } else {
            None
        }
    } else {
        None
    };

    let mut terminal = ratatui::init();
    let result = tui::run(&mut terminal, api, store, session);
    ratatui::restore();

    result
}
