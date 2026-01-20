//! Command-line interface.

mod commands;
pub mod install;
mod interactive;

pub use commands::Cli;
pub use interactive::run_interactive_config;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    cursor, event, execute,
    terminal::{disable_raw_mode, is_raw_mode_enabled, LeaveAlternateScreen},
};
use std::io;

pub fn run() -> Result<()> {
    // Set up global panic hook to restore terminal state on panic
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Only try to restore if we seem to be in raw mode (TUI active)
        if let Ok(true) = is_raw_mode_enabled() {
            let _ = disable_raw_mode();
            let _ = execute!(
                io::stdout(),
                LeaveAlternateScreen,
                event::DisableMouseCapture,
                cursor::Show
            );
        }
        default_hook(info);
    }));

    let cli = Cli::parse();
    cli.run()
}
