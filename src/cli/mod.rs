//! Command line: argument parsing, dispatch and output conventions.
//!
//! Contract: nothing prompts; `--json` on every command (schema `"v": 1`);
//! exit 0 ok (including "nothing changed"), 1 error, 2 usage, 3 conflict.

mod admin;
mod out;
mod query;
mod record;

use crate::error::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::io::IsTerminal;
use std::path::PathBuf;

pub use out::Out;
pub use query::recap_text;

#[derive(Parser, Debug)]
#[clap(
    name = "vs",
    version,
    about = "vibesnap: an edit ledger for AI agents. Records every file change, who made it and why; query it, undo it, commit it.",
    after_help = "Refs: @ (working tree), @-N, e:N, turn, turn-N, turn:N, session, agent:<name>, t:14:05, t:-10m, <label>, git:<rev>, A..B; files: path@ref"
)]
pub struct Cli {
    /// Print JSON (schema "v": 1); `log` prints one JSON object per line
    #[clap(long, global = true)]
    pub json: bool,
    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create a store in a directory and record its current state
    Init {
        #[clap(default_value = ".")]
        path: PathBuf,
    },
    /// Record the working tree now as a user event, optionally labelled
    Snap {
        #[clap(short, long)]
        message: Option<String>,
        /// Also pin the result under this label
        #[clap(short, long)]
        label: Option<String>,
    },
    /// What changed since the last recorded event (does not record anything)
    Status,
    /// Changes between two states. No refs: this turn; one ref: that ref against the working tree
    Diff {
        refs: Vec<String>,
        /// Files with +/- line counts and who changed them
        #[clap(long)]
        stat: bool,
        /// Only the paths
        #[clap(long)]
        name_only: bool,
        /// Lines of context around each hunk
        #[clap(short = 'U', long, default_value_t = 3)]
        unified: usize,
        #[clap(long, default_value_t = 400)]
        max_lines: usize,
        #[clap(last = true)]
        paths: Vec<PathBuf>,
    },
    /// Events, newest first
    Log {
        /// A ref, range (A..B), turn, agent:<name> or e:N to select events
        spec: Option<String>,
        /// Only events by this agent (subagent type or id, or "main")
        #[clap(long)]
        agent: Option<String>,
        /// Only events by this actor: agent, user or external
        #[clap(long)]
        actor: Option<String>,
        /// Only events after this ref
        #[clap(long)]
        since: Option<String>,
        /// Only events that added or removed this string
        #[clap(short = 'S')]
        search: Option<String>,
        /// Show the files each event changed
        #[clap(long)]
        stat: bool,
        #[clap(short = 'n', long, default_value_t = 30)]
        limit: usize,
        #[clap(last = true)]
        paths: Vec<PathBuf>,
    },
    /// A file's content at a state (path@ref), or one event's details (e:N)
    Show {
        spec: String,
        /// With e:N, include the patch
        #[clap(short, long)]
        patch: bool,
        #[clap(long, default_value_t = 400)]
        max_lines: usize,
    },
    /// Which event, agent and turn last changed each line of a file
    Blame {
        path: PathBuf,
        /// Line range a,b (1-based, inclusive)
        #[clap(short = 'L')]
        lines: Option<String>,
    },
    /// Revert events (e:N, @-1, turn, agent:<name>, A..B) by a three-way inverse; keeps later edits
    Undo {
        spec: String,
        /// Also revert events whose authorship is ambiguous
        #[clap(long)]
        include_ambiguous: bool,
        /// Also revert changes made outside the agent (editor, other tools); implied by e:N
        #[clap(long)]
        include_external: bool,
        /// Show what would change without writing
        #[clap(long)]
        dry_run: bool,
        #[clap(last = true)]
        paths: Vec<PathBuf>,
    },
    /// Bring the working tree (or some paths) to a state; the current state is recorded first
    Restore {
        r#ref: String,
        #[clap(long)]
        dry_run: bool,
        #[clap(last = true)]
        paths: Vec<PathBuf>,
    },
    /// Pin a state under a name so gc keeps it; no arguments lists labels
    Label {
        name: Option<String>,
        /// The state to label (default: the working tree, recorded now)
        r#ref: Option<String>,
        /// Delete the label
        #[clap(short, long)]
        delete: bool,
        /// Move an existing label
        #[clap(short, long)]
        force: bool,
    },
    /// Compact summary of this session's turns (or the last turn) for re-orienting
    Recap {
        #[clap(long)]
        turn: bool,
        #[clap(long, default_value_t = 40)]
        max_lines: usize,
    },
    /// Lines one agent wrote that another agent then changed
    Overlaps {
        #[clap(long, default_value = "turn")]
        since: String,
    },
    /// Commit the files agents changed in a span (default: this turn) with git
    Commit {
        #[clap(default_value = "turn")]
        span: String,
        #[clap(short, long)]
        message: Option<String>,
        /// Include files the user also edited during the span
        #[clap(long)]
        include_mixed: bool,
        /// Leave these paths out
        #[clap(long)]
        exclude: Vec<PathBuf>,
        #[clap(long)]
        dry_run: bool,
    },
    /// Drop unlabelled history older than the retention window and repack
    Gc {
        /// Retention, e.g. 7d or 12h (default from config: 7d)
        #[clap(long)]
        keep: Option<String>,
    },
    /// Rebuild the SQLite index from the ledger
    Reindex,
    /// Check the store for missing objects, a stale index and hook errors
    Doctor,
    /// Internal: handle an agent hook (JSON on stdin)
    Hook { agent: String },
    /// Install the hooks and skill for an agent (claude)
    Install {
        agent: String,
        /// Into this project's .claude/ instead of ~/.claude/
        #[clap(long)]
        project: bool,
        #[clap(long)]
        uninstall: bool,
    },
    /// Delete the store and all history
    Reset {
        #[clap(long)]
        confirm: bool,
    },
}

pub fn main() {
    // Exit quietly when the reader goes away (`vs log | head`), like other CLIs.
    // SAFETY: restoring the default disposition of a signal at start-up.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    if cli.json || !std::io::stdout().is_terminal() {
        colored::control::set_override(false);
    }
    let out = Out::new(cli.json);
    if let Command::Hook { agent } = &cli.command {
        std::process::exit(crate::hook::run(agent));
    }
    if let Err(e) = run(cli.command, &out) {
        if out.json {
            eprintln!(
                "{}",
                serde_json::json!({"v": 1, "error": {"kind": e.kind(), "message": e.to_string()}})
            );
        } else {
            eprintln!("{}", e.to_string().red());
        }
        std::process::exit(e.exit_code());
    }
}

fn run(command: Command, out: &Out) -> Result<()> {
    match command {
        Command::Init { path } => record::init(out, &path),
        Command::Snap { message, label } => record::snap(out, message, label),
        Command::Status => record::status(out),
        Command::Diff {
            refs,
            stat,
            name_only,
            unified,
            max_lines,
            paths,
        } => query::diff(out, &refs, stat, name_only, unified, max_lines, &paths),
        Command::Log {
            spec,
            agent,
            actor,
            since,
            search,
            stat,
            limit,
            paths,
        } => query::log(
            out,
            query::LogArgs {
                spec,
                agent,
                actor,
                since,
                search,
                stat,
                limit,
                paths,
            },
        ),
        Command::Show {
            spec,
            patch,
            max_lines,
        } => query::show(out, &spec, patch, max_lines),
        Command::Blame { path, lines } => query::blame(out, &path, lines.as_deref()),
        Command::Undo {
            spec,
            include_ambiguous,
            include_external,
            dry_run,
            paths,
        } => record::undo(
            out,
            &spec,
            include_ambiguous,
            include_external,
            dry_run,
            &paths,
        ),
        Command::Restore {
            r#ref,
            dry_run,
            paths,
        } => record::restore(out, &r#ref, dry_run, &paths),
        Command::Label {
            name,
            r#ref,
            delete,
            force,
        } => record::label(out, name, r#ref, delete, force),
        Command::Recap { turn, max_lines } => query::recap(out, turn, max_lines),
        Command::Overlaps { since } => query::overlaps(out, &since),
        Command::Commit {
            span,
            message,
            include_mixed,
            exclude,
            dry_run,
        } => admin::commit(out, &span, message, include_mixed, &exclude, dry_run),
        Command::Gc { keep } => admin::gc(out, keep),
        Command::Reindex => admin::reindex(out),
        Command::Doctor => admin::doctor(out),
        Command::Hook { .. } => unreachable!(),
        Command::Install {
            agent,
            project,
            uninstall,
        } => admin::install(out, &agent, project, uninstall),
        Command::Reset { confirm } => record::reset(out, confirm),
    }
}
