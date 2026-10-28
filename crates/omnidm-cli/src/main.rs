// SPDX-License-Identifier: GPL-3.0-or-later

//! Command-line client for OmniDm.
//!
//! When the OmniDm app is running, downloads go through it (and appear in its window).
//! Otherwise, or with `--standalone`, or when `OMNIDM_DATA_DIR` names a database of its
//! own, the command downloads by itself.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use omnidm_cli::{
    CliError, DATABASE_FILE, GetOptions, GetResult, data_dir, format_bytes, get, get_via_app,
    list_via_app, parse_header, progress_line,
};
use omnidm_core::engine::Event;
use omnidm_store::Store;
use tokio_util::sync::CancellationToken;

/// Exit code after Ctrl+C, as shells use for an interrupted command.
const EXIT_PAUSED: u8 = 130;

#[derive(Parser)]
#[command(name = "omnidm", version, about = "OmniDm download manager")]
struct Cli {
    /// Download in this program even when the OmniDm app is running.
    #[arg(long, global = true)]
    standalone: bool,
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
        /// Cookies for the site, e.g. -b "session=abc; theme=dark". Repeatable.
        #[arg(short = 'b', long = "cookie")]
        cookies: Vec<String>,
        /// Form data to POST, e.g. --data "id=7&agree=1". Sent as a web form unless a
        /// Content-Type header is given.
        #[arg(long)]
        data: Option<String>,
    },
    /// List downloads, oldest first.
    List,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let app = if cli.standalone || std::env::var_os("OMNIDM_DATA_DIR").is_some() {
        None
    } else {
        match connect_to_app().await {
            Ok(app) => app,
            Err(e) => {
                eprintln!("Error: {e}");
                return ExitCode::FAILURE;
            }
        }
    };
    match cli.command {
        Command::Get {
            url,
            output,
            dir,
            connections,
            headers,
            cookies,
            data,
        } => {
            let opts = GetOptions {
                url,
                output,
                dir,
                connections,
                headers,
                cookies,
                data,
            };
            run_get(opts, app).await
        }
        Command::List => match app {
            Some(app) => list_from_app(app).await,
            None => list_standalone(),
        },
    }
}

#[cfg(windows)]
type AppConnection = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(not(windows))]
type AppConnection = tokio::io::DuplexStream;

/// The running app, if there is one next to this program. Only an OmniDm app found next
/// to the CLI is trusted to serve the pipe.
#[cfg(windows)]
async fn connect_to_app() -> Result<Option<AppConnection>, String> {
    use omnidm_ipc::pipe::{PipeError, connect, default_pipe_name};

    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let apps = omnidm_ipc::app_paths(&me);
    if apps.is_empty() {
        return Ok(None);
    }
    // OMNIDM_PIPE is for development and tests.
    let name = std::env::var("OMNIDM_PIPE").unwrap_or_else(|_| default_pipe_name());
    match connect(&name, &apps).await {
        Ok(pipe) => Ok(Some(pipe)),
        Err(PipeError::NotRunning) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(not(windows))]
async fn connect_to_app() -> Result<Option<AppConnection>, String> {
    Ok(None)
}

/// Prints one updating status line on a terminal.
struct ProgressLine {
    show: bool,
    last_len: usize,
}

impl ProgressLine {
    fn new() -> Self {
        Self {
            show: std::io::stderr().is_terminal(),
            last_len: 0,
        }
    }

    fn update(&mut self, downloaded: u64, total: Option<u64>, speed: u64, connections: u32) {
        if !self.show {
            return;
        }
        let line = progress_line(downloaded, total, speed, connections);
        let width = self.last_len;
        eprint!("\r{line:<width$}");
        let _ = std::io::stderr().flush();
        self.last_len = line.len();
    }

    fn finish(&self) {
        if self.show && self.last_len > 0 {
            eprintln!();
        }
    }
}

async fn run_get(opts: GetOptions, app: Option<AppConnection>) -> ExitCode {
    let cancel = CancellationToken::new();
    let on_ctrl_c = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_ctrl_c.cancel();
        }
    });

    let mut line = ProgressLine::new();
    let result = match app {
        Some(app) => {
            get_via_app(app, opts, cancel, |p| {
                line.update(p.downloaded, p.total, p.speed, p.connections);
            })
            .await
        }
        None => {
            get(opts, &data_dir(), cancel, |event| {
                if let Event::Progress {
                    downloaded,
                    total,
                    speed,
                    connections,
                } = *event
                {
                    line.update(downloaded, total, speed, connections);
                }
            })
            .await
        }
    };
    line.finish();
    report(result)
}

fn report(result: Result<GetResult, CliError>) -> ExitCode {
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

async fn list_from_app(app: AppConnection) -> ExitCode {
    match list_via_app(app).await {
        Ok(downloads) => {
            if downloads.is_empty() {
                println!("No downloads yet.");
            }
            for d in downloads {
                let size = d.total.map_or_else(|| "?".to_string(), format_bytes);
                let name = d.file_name.as_deref().unwrap_or("?");
                println!(
                    "{:>4}  {:<9}  {size:>10}  {name}  {}",
                    d.download_id, d.status, d.url
                );
                if let Some(error) = d.error {
                    println!("      {error}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn list_standalone() -> ExitCode {
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
        let status = d.status.as_str();
        println!("{:>4}  {status:<9}  {size:>10}  {name}  {}", d.id.0, d.url);
        if let Some(error) = d.error {
            println!("      {error}");
        }
    }
    ExitCode::SUCCESS
}
