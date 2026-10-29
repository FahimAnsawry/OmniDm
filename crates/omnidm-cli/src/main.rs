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
    list_via_app, parse_header, progress_line, request,
};
use omnidm_core::engine::Event;
use omnidm_ipc::protocol::{
    CategoryFolder, DownloadIds, Message, NewQueue, QueueEdit, QueueItems, QueueMove, QueueRef,
    QueueSchedule,
};
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
        /// Add it to this queue of the app instead of downloading now.
        #[arg(short, long)]
        queue: Option<String>,
    },
    /// List downloads, oldest first.
    List,
    /// Manage the app's queues: downloads that run in order, a few at a time.
    Queue {
        #[command(subcommand)]
        action: QueueCommand,
    },
    /// See the app's categories, and choose where their browser downloads go.
    Category {
        #[command(subcommand)]
        action: CategoryCommand,
    },
}

#[derive(Subcommand)]
enum CategoryCommand {
    /// List the categories, in the order they are tried, with their folders.
    List,
    /// Save a category's browser downloads in this folder. Without one, they go to the
    /// downloads folder.
    Folder {
        name: String,
        folder: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum QueueCommand {
    /// List the queues and their downloads.
    List,
    /// Make a new queue.
    Create {
        name: String,
        /// Downloads running at once (1–16, default 3).
        #[arg(short, long)]
        max: Option<u32>,
        /// What to do when it finishes: none, sleep, hibernate, shutdown or quit.
        #[arg(long)]
        then: Option<String>,
    },
    /// Change a queue.
    Edit {
        name: String,
        #[arg(long)]
        rename: Option<String>,
        /// Downloads running at once (1–16).
        #[arg(short, long)]
        max: Option<u32>,
        /// What to do when it finishes: none, sleep, hibernate, shutdown or quit.
        #[arg(long)]
        then: Option<String>,
    },
    /// Set when a queue starts and stops by itself (replacing its old schedule). With no
    /// options, it only starts by hand.
    Schedule {
        name: String,
        /// Start every chosen day at this time (HH:MM, 24-hour).
        #[arg(long, value_name = "HH:MM", conflicts_with = "once")]
        at: Option<String>,
        /// With --at: daily (default), weekdays, weekend, or days like mon,wed,fri.
        #[arg(long, requires = "at")]
        days: Option<String>,
        /// Start once at this date and time.
        #[arg(long, value_name = "YYYY-MM-DD HH:MM")]
        once: Option<String>,
        /// Stop the queue (pausing its downloads) at this time (HH:MM).
        #[arg(long, value_name = "HH:MM")]
        stop: Option<String>,
        /// Wake the PC from sleep to start.
        #[arg(long)]
        wake: bool,
    },
    /// Delete a queue. Its downloads stay, in no queue.
    Delete { name: String },
    /// Start a queue: it downloads everything unfinished in it, in order.
    Start { name: String },
    /// Stop a queue: its downloads pause.
    Stop { name: String },
    /// Put downloads (numbers from `omnidm list`) at the end of a queue.
    Add {
        name: String,
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// Take downloads out of their queue.
    Remove {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// Move a download to a place in its queue (1 = first).
    Move { id: i64, position: u32 },
    /// Cancel the sleep, shutdown or quit counting down after a queue finished.
    CancelAction,
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
            queue,
        } => {
            let opts = GetOptions {
                url,
                output,
                dir,
                connections,
                headers,
                cookies,
                data,
                queue,
            };
            run_get(opts, app).await
        }
        Command::List => match app {
            Some(app) => list_from_app(app).await,
            None => list_standalone(),
        },
        Command::Queue { action } => match app {
            Some(app) => run_queue(action, app).await,
            None => {
                eprintln!("Error: queues belong to the OmniDm app, and it isn't running");
                ExitCode::FAILURE
            }
        },
        Command::Category { action } => match app {
            Some(app) => run_category(action, app).await,
            None => {
                eprintln!("Error: categories belong to the OmniDm app, and it isn't running");
                ExitCode::FAILURE
            }
        },
    }
}

async fn run_category(action: CategoryCommand, app: AppConnection) -> ExitCode {
    let (message, done) = match action {
        CategoryCommand::List => (Message::ListCategories, String::new()),
        CategoryCommand::Folder { name, folder } => {
            // A relative folder means one under the current folder, as everywhere else.
            let folder = match folder.map(std::path::absolute).transpose() {
                Ok(folder) => folder,
                Err(e) => {
                    eprintln!("Error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let done = match &folder {
                Some(folder) => format!("{name} downloads now go to {}.", folder.display()),
                None => format!("{name} downloads now go to the downloads folder."),
            };
            let folder = folder.map(|f| f.to_string_lossy().into_owned());
            (
                Message::SetCategoryFolder(CategoryFolder {
                    category: name,
                    folder,
                }),
                done,
            )
        }
    };
    match request(app, message).await {
        Ok(Message::Categories(categories)) => {
            for c in categories.items {
                let folder = c.folder.as_deref().unwrap_or("(downloads folder)");
                println!("{}  →  {folder}", c.name);
                let mut rules: Vec<String> = c.extensions.iter().map(|e| format!(".{e}")).collect();
                rules.extend(c.mime_types);
                rules.extend(c.hosts);
                println!("      {}", rules.join(" "));
            }
            ExitCode::SUCCESS
        }
        Ok(_) => {
            println!("{done}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// "starts 02:00 weekdays, stops 07:00, wakes the PC (next start 2026-10-30 02:00)".
fn describe(schedule: &QueueSchedule, next_start: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(at) = &schedule.start_at {
        let days = schedule.days.as_deref().unwrap_or("daily");
        parts.push(format!("starts {at} {days}"));
    }
    if let Some(once) = &schedule.once {
        parts.push(format!("starts once at {once}"));
    }
    if let Some(stop) = &schedule.stop_at {
        parts.push(format!("stops {stop}"));
    }
    if schedule.wake {
        parts.push("wakes the PC".into());
    }
    let mut text = parts.join(", ");
    if let Some(next) = next_start {
        text.push_str(&format!(" (next start {next})"));
    }
    text
}

async fn run_queue(action: QueueCommand, app: AppConnection) -> ExitCode {
    let (message, done) = match action {
        QueueCommand::List => (Message::ListQueues, String::new()),
        QueueCommand::Create { name, max, then } => (
            Message::CreateQueue(NewQueue {
                name: name.clone(),
                max_concurrent: max,
                on_complete: then,
            }),
            format!("Made queue {name}."),
        ),
        QueueCommand::Edit {
            name,
            rename,
            max,
            then,
        } => (
            Message::EditQueue(QueueEdit {
                queue: name.clone(),
                rename,
                max_concurrent: max,
                on_complete: then,
                schedule: None,
            }),
            format!("Changed queue {name}."),
        ),
        QueueCommand::Schedule {
            name,
            at,
            days,
            once,
            stop,
            wake,
        } => {
            let off = at.is_none() && once.is_none() && stop.is_none();
            (
                Message::EditQueue(QueueEdit {
                    queue: name.clone(),
                    rename: None,
                    max_concurrent: None,
                    on_complete: None,
                    schedule: Some(QueueSchedule {
                        start_at: at,
                        days,
                        once,
                        stop_at: stop,
                        wake,
                    }),
                }),
                if off {
                    format!("Queue {name} now starts only by hand.")
                } else {
                    format!("Scheduled queue {name}. See it with: omnidm queue list")
                },
            )
        }
        QueueCommand::Delete { name } => (
            Message::DeleteQueue(QueueRef {
                queue: name.clone(),
            }),
            format!("Deleted queue {name}."),
        ),
        QueueCommand::Start { name } => (
            Message::StartQueue(QueueRef {
                queue: name.clone(),
            }),
            format!("Started queue {name}."),
        ),
        QueueCommand::Stop { name } => (
            Message::StopQueue(QueueRef {
                queue: name.clone(),
            }),
            format!("Stopped queue {name}."),
        ),
        QueueCommand::Add { name, ids } => (
            Message::QueueAdd(QueueItems {
                queue: name.clone(),
                download_ids: ids,
            }),
            format!("Added to queue {name}."),
        ),
        QueueCommand::Remove { ids } => (
            Message::QueueRemove(DownloadIds { download_ids: ids }),
            "Taken out of their queue.".into(),
        ),
        QueueCommand::Move { id, position } => (
            Message::QueueMove(QueueMove {
                download_id: id,
                index: position.saturating_sub(1),
            }),
            format!("Moved #{id}."),
        ),
        QueueCommand::CancelAction => (Message::CancelAfterQueue, "Cancelled.".into()),
    };
    match request(app, message).await {
        Ok(Message::Queues(queues)) => {
            for q in queues.items {
                let state = if q.running { "running" } else { "stopped" };
                println!(
                    "{}  ({state}, {} at a time, then: {})",
                    q.name, q.max_concurrent, q.on_complete
                );
                if let Some(schedule) = &q.schedule {
                    println!("      {}", describe(schedule, q.next_start.as_deref()));
                }
                if q.download_ids.is_empty() {
                    println!("      (empty)");
                } else {
                    let ids: Vec<String> =
                        q.download_ids.iter().map(|id| format!("#{id}")).collect();
                    println!("      {}", ids.join(" "));
                }
            }
            ExitCode::SUCCESS
        }
        Ok(_) => {
            println!("{done}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
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
        Ok(GetResult::Queued(id)) => {
            println!("Added #{id} to the queue. It downloads when the queue runs.");
            ExitCode::SUCCESS
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
                let queue = d.queue.map(|q| format!("  [{q}]")).unwrap_or_default();
                let category = d.category.map(|c| format!("  ({c})")).unwrap_or_default();
                println!(
                    "{:>4}  {:<9}  {size:>10}  {name}{category}{queue}  {}",
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
