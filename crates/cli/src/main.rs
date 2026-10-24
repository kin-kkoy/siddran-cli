//! Siddran in the terminal. One binary, two modes: a subcommand runs and exits,
//! bare `siddran` will launch the TUI (Phase 3). Everything real lives in
//! `siddran-core`, so both halves ask the data the same questions.
//!
//! Phase 1 is read-only by design: no writes at all until the fidelity contract
//! is extended with atomic writes, a lock and reconcile.

mod bagpath;
mod config;
mod cmd;
mod mcp;
mod out;
mod session;
mod session_state;
mod tui;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "siddran",
    version,
    about = "Notes, tasks and calendar from your Bag — in the terminal",
    disable_help_subcommand = true
)]
struct Cli {
    /// Path to the Bag. Defaults to $SIDDRAN_BAG, then ~/.config/siddran/config.toml.
    #[arg(long, global = true, value_name = "PATH")]
    bag: Option<PathBuf>,

    /// Emit JSON instead of a table.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Notes and notebooks
    #[command(subcommand)]
    Note(cmd::note::Cmd),
    /// Notebooks
    #[command(subcommand)]
    Notebook(cmd::note::NotebookCmd),
    /// Standalone tasks
    #[command(subcommand)]
    Task(cmd::task::Cmd),
    /// Daily tasks, recurring or ephemeral
    #[command(subcommand)]
    Daily(cmd::task::DailyCmd),
    /// Projects (bundles) and their subtasks
    #[command(subcommand)]
    Project(cmd::task::ProjectCmd),
    /// Calendar events
    #[command(subcommand)]
    Event(cmd::event::Cmd),
    /// Settings
    #[command(subcommand)]
    Settings(cmd::settings::Cmd),
    /// The Bag itself
    #[command(subcommand)]
    Bag(cmd::bag::Cmd),
    /// Serve the Bag to an AI over MCP (stdio). Point your AI tool at `siddran mcp`.
    Mcp,
    /// List every AI tool (the same ones `siddran mcp` serves). Start here.
    Tools,
    /// Run one AI tool: `siddran call note_edit '{"note":"Ideas","append":"- more"}'`.
    /// JSON arguments as the second argument, or on stdin with `-`.
    Call {
        tool: String,
        /// JSON arguments; `-` reads them from stdin; omit for none.
        args: Option<String>,
    },
}

fn launch_tui(bag_arg: Option<PathBuf>) -> Result<(), String> {
    let root = bagpath::resolve(bag_arg)?;
    // The TUI stays open for hours, so it holds no lock while it sits there —
    // it takes one per write. See `Session`.
    let (sess, bag) = session::Session::begin_as(&root, "tui")?;
    // Whatever you last chose with P/F, or the defaults.
    let palette = match config::get("palette").as_deref() {
        Some("yazi") => tui::Palette::Yazi,
        _ => tui::Palette::Ember,
    };
    let frame = tui::Frame::parse(config::get("frame").as_deref().unwrap_or("rounded"));
    let mut app = tui::App::new(bag, root.clone(), sess, palette, frame, bagpath::editor());

    // The desktop's "remember note state" toggle, honoured here as "put every
    // workspace back where it was".
    if app.remember_state() {
        if let Some(r) = session_state::load(&root) {
            app.restore(&r);
        }
    }

    app.rearm_alarms();

    let remember = app.remember_state();
    let result = tui::run(&mut app);
    if remember {
        session_state::save(&root, &app.snapshot());
    }
    result.map_err(|e| e.to_string())
}

impl Command {
    /// Whether this command writes. Drives whether the lock is taken at all.
    fn mutates(&self) -> bool {
        match self {
            Command::Task(c) => c.mutates(),
            Command::Note(c) => c.mutates(),
            Command::Bag(c) => c.mutates(),
            Command::Daily(c) => c.mutates(),
            Command::Project(c) => c.mutates(),
            Command::Event(_) | Command::Notebook(_) | Command::Settings(_) => false,
            // Locks per tool call, not for its lifetime — see mcp/mod.rs.
            Command::Mcp | Command::Tools | Command::Call { .. } => false,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Bare `siddran` launches the TUI.
    let Some(command) = cli.command else {
        return match launch_tui(cli.bag) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        };
    };

    // `tools` needs no Bag at all.
    if let Command::Tools = command {
        mcp::list_tools(cli.json);
        return ExitCode::SUCCESS;
    }

    let root = match bagpath::resolve(cli.bag) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    // A long-lived server: no session of its own, it takes the lock per call.
    if let Command::Mcp = command {
        return match mcp::run(root) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        };
    }

    // One tool call, the MCP server's code path without the protocol: it locks
    // per write itself, like the server does.
    if let Command::Call { tool, args } = command {
        let args = match args.as_deref() {
            Some("-") => {
                let mut s = String::new();
                use std::io::Read;
                if std::io::stdin().read_to_string(&mut s).is_err() {
                    eprintln!("error: could not read the arguments from stdin");
                    return ExitCode::from(2);
                }
                s
            }
            Some(a) => a.to_owned(),
            None => String::new(),
        };
        return match mcp::call_tool(&root, &tool, &args) {
            Ok(out) => {
                println!("{out}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(1)
            }
        };
    }

    // Only mutating commands take the lock. A read while a TUI holds the Bag
    // must still work — that is precisely when you want `siddran task ls`.
    let (sess, bag) = if command.mutates() {
        match session::Session::begin(&root) {
            Ok((s, b)) => (Some(s), b),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(3);
            }
        }
    } else {
        match siddran_core::Bag::open(&root) {
            Ok(b) => (None, b),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        }
    };

    let ctx = cmd::Ctx {
        bag,
        root,
        session: sess,
        mode: out::Mode::pick(cli.json),
    };

    let found = match command {
        Command::Note(c) => cmd::note::run(c, &ctx),
        Command::Notebook(c) => cmd::note::run_notebook(c, &ctx),
        Command::Task(c) => cmd::task::run(c, &ctx),
        Command::Daily(c) => cmd::task::run_daily(c, &ctx),
        Command::Project(c) => cmd::task::run_project(c, &ctx),
        Command::Event(c) => cmd::event::run(c, &ctx),
        Command::Settings(c) => cmd::settings::run(c, &ctx),
        Command::Bag(c) => cmd::bag::run(c, &ctx),
        Command::Mcp | Command::Tools | Command::Call { .. } => unreachable!("served above, before a session is opened"),
    };

    // Exit 1 for "asked for something specific and it is not there", so shell
    // conditionals work: `siddran note cat X >/dev/null || echo missing`.
    if found {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
