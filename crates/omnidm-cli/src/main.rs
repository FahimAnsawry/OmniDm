// SPDX-License-Identifier: GPL-3.0-or-later

//! Command-line client for OmniDm.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use omnidm_cli::{
    DATABASE_FILE, GetOptions, GetResult, data_dir, format_bytes, get, parse_header, progress_line,
};
use omnidm_core::engine::Event;
use omnidm_store::Store;
use tokio_util::sync::CancellationToken;

/// Exit code after Ctrl+C, as shells use for an interrupted command.
const EXIT_PAUSED: u8 = 130;

#[derive(Parser)]
#[command(name = "omnidm", version, about = "OmniDm download manager")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download a file. Ctrl+C pauses; run the same command again to resume.
    Get {
        url: String,
        /// File name to save as (default: taken from the URL).
        #[arg(short, long)]
        output: Option<String>,
        /// Folder to save into.
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,
        /// Maximum connections (1–32).
        #[arg(short, long, default_value_t = 8)]
        connections: u32,
        /// Extra request header, e.g. -H "Referer: https://example.com". Repeatable.
        #[arg(short = 'H', long = "header", value_parser = parse_header)]
        headers: Vec<(String, String)>,
    },
    /// List downloads, oldest first.
    List,
}

#[tokio::main]
async fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Get {
            url,
            output,
            dir,
            connections,
            headers,
        } => {
            run_get(GetOptions {
                url,
                output,
                dir,
                connections,
                headers,
            })
            .await
        }
        Command::List => run_list(),
    }
}

async fn run_get(opts: GetOptions) -> ExitCode {
    let cancel = CancellationToken::new();
    let on_ctrl_c = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_ctrl_c.cancel();
        }
    });

    let show_progress = std::io::stderr().is_terminal();
    let mut last_len = 0;
    let result = get(opts, &data_dir(), cancel, |event| {
        if let Event::Progress {
            downloaded,
            total,
            speed,
            connections,
        } = *event
            && show_progress
        {
            let line = progress_line(downloaded, total, speed, connections);
            eprint!("\r{line:<last_len$}");
            let _ = std::io::stderr().flush();
            last_len = line.len();
        }
    })
    .await;
    if show_progress && last_len > 0 {
        eprintln!();
    }

    match result {
        Ok(GetResult::Completed(path)) => {
            println!("Saved to {}", path.display());
            ExitCode::SUCCESS
        }
        Ok(GetResult::Paused) => {
            eprintln!("Paused. Run the same command again to resume.");
            ExitCode::from(EXIT_PAUSED)
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_list() -> ExitCode {
    let path = data_dir().join(DATABASE_FILE);
    if !path.exists() {
        println!("No downloads yet.");
        return ExitCode::SUCCESS;
    }
    let downloads = match Store::open(&path).and_then(|store| store.list_downloads()) {
        Ok(downloads) => downloads,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if downloads.is_empty() {
        println!("No downloads yet.");
    }
    for d in downloads {
        let size = d.total_size.map_or_else(|| "?".to_string(), format_bytes);
        let name = d.file_name.as_deref().unwrap_or("?");
        let status = format!("{:?}", d.status).to_lowercase();
        println!("{:>4}  {status:<9}  {size:>10}  {name}  {}", d.id.0, d.url);
        if let Some(error) = d.error {
            println!("      {error}");
        }
    }
    ExitCode::SUCCESS
}
