# vibesnap

An edit ledger for AI agents.

vibesnap records every change to your project as it happens: Claude's edits, its subagents' edits, shell commands, and your own edits in an editor. Each change is an event that says who made it, in which turn, with which tool. Agents (and you) can then ask the history questions instead of guessing:

```
$ vs diff turn --stat                  # what did we change since the last prompt?
src/auth.ts      +12 -3   main, test-writer
tests/auth.ts    +40 -0   test-writer

$ vs blame src/auth.ts -L 40,60        # who wrote these lines, and when?
40-52    e:815     test-writer     14:05:12  turn:14
53-60    e:798     main            13:58:40  turn:13

$ vs undo agent:test-writer            # revert one subagent's work, keep everything else
```

Nothing has to remember to snapshot. Hooks record changes; the history is queried only when someone needs to look back.

## Install

```bash
cargo install vibesnap          # installs `vs` and `vibesnap` (same binary)
```

Then hook it into Claude Code, either as a plugin:

```bash
claude plugin marketplace add kirillleventcov/vibesnap
claude plugin install vibesnap@vibesnap
```

or directly into your settings (`~/.claude`, or the project's `.claude` with `--project`):

```bash
vs install claude
```

Both install the hooks and a skill that tells Claude when and how to look back. From then on, a Claude Code session in a git repository creates `.vibe/` at the repository's top level and records everything. Outside git, run `vs init` yourself.

Linux and macOS only.

## Using it

Most of the time you do nothing. When you want to look back:

| Question | Command |
| --- | --- |
| What changed since the last recorded event? | `vs status` |
| What did this turn change? | `vs diff turn --stat`, then `vs diff turn -- <path>` |
| What did the file look like before the session? | `vs show src/auth.ts@session` |
| ...at 14:05, or 3 events ago? | `vs show src/auth.ts@t:14:05`, `vs show src/auth.ts@-3` |
| Who changed lines 40-60? | `vs blame src/auth.ts -L 40,60` |
| What did a subagent touch? | `vs log --agent test-writer --stat` |
| Did the user edit anything while the agent worked? | `vs log --actor external --since session` |
| When did `parseToken` disappear? | `vs log -S parseToken -- src/` |
| Undo the last edit, this turn, or one subagent | `vs undo @-1`, `vs undo turn`, `vs undo agent:test-writer` |
| Go back to a point in time | `vs restore t:14:05`, `vs restore before-refactor` |
| Did two agents overwrite each other? | `vs overlaps --since turn` |
| Re-orient after context compaction | `vs recap` |
| Commit what the agents did this turn | `vs commit turn -m "..."` |

`undo` applies the inverse of the selected events as a three-way merge, so later edits by others survive. If a later edit touched the same lines it refuses (exit code 3) and names the event. `undo` and `restore` record the current state first, so both can be undone.

### Refs

| Ref | Means |
| --- | --- |
| `@` | The working tree now |
| `@-N` | N events back (`@-1`: before the last event) |
| `e:812` | Right after event 812 |
| `turn`, `turn-2`, `turn:14` | Start of the latest turn, of two turns before it, or of turn number 14 |
| `session`, `session:<id>` | Start of the latest session, or of a named one |
| `agent:<type or id>` | Start of that subagent's latest run (in `undo`/`log`: its events) |
| `t:14:05`, `t:-10m`, `t:2026-10-16T14:05` | The state at a time |
| `<label>` | A label made with `vs label` or `vs snap --label` |
| `git:HEAD`, `git:<rev>` | A git commit's tree |
| `A..B` | A range, for `diff`, `log` and `undo` |
| `path@ref` | A file at a state, e.g. `src/auth.ts@turn`; `path@@` is the working tree |

### Commands

| Command | Purpose |
| --- | --- |
| `vs status` | Changes since the last event; queued hook events; recent hook errors |
| `vs diff [A] [B] [--stat\|--name-only] [-- paths]` | No refs: this turn against the working tree. One ref: that ref against the working tree |
| `vs log [spec] [--agent] [--actor] [--since] [-S text] [--stat] [-- paths]` | Events, newest first |
| `vs show <path@ref>`, `vs show e:N [-p]` | A file at a state, or one event |
| `vs blame <path> [-L a,b]` | Event, agent, time and turn for each line |
| `vs undo <spec> [-- paths]` | Revert events by a three-way inverse |
| `vs restore <ref> [-- paths]` | Bring files to a state; writes only what differs, deletes what the state lacks |
| `vs label [name [ref]]`, `vs label -d name` | Pin a state so gc keeps it; no arguments lists labels |
| `vs snap [-m msg] [--label name]` | Record now as a user event |
| `vs recap [--turn]` | Per-turn summary: files, agents, subagent summaries (40 lines) |
| `vs overlaps [--since ref]` | Lines one agent wrote that another then changed |
| `vs commit [span] [-m msg] [--include-mixed] [--exclude path]` | Commit only the files agents changed, through `git commit` so your git hooks run |
| `vs gc [--keep 7d]` | Drop unlabelled history past the retention window and repack |
| `vs reindex`, `vs doctor` | Rebuild the index from the ledger; check the store |
| `vs init [path]`, `vs reset --confirm` | Create or delete a store |
| `vs install claude [--project] [--uninstall]` | Install the hooks and skill |
| `vs hook claude` | Internal: handles one hook payload on stdin |

For scripts and agents: nothing prompts; every command takes `--json` (one object with `"v": 1`; `log` prints one per line); paths resolve against the current directory; exit codes are 0 (ok, including "nothing changed"), 1 (error), 2 (usage) and 3 (refused because of a conflict). Diffs and file contents stop at `--max-lines` (default 400) and say how to see more.

## How it works

`.vibe/` is a bare git repository written in process through [gitoxide](https://github.com/GitoxideLabs/gitoxide), plus a SQLite index:

```
.vibe/
  objects/      git objects; objects/info/alternates points at .git/objects
  refs/vibesnap/ledger         one commit per event; its tree is the state after the event
  refs/vibesnap/labels/<name>  labels
  index.db      SQLite: events, paths per event, turns/sessions/subagents, stat cache
  hook.log      hook errors; inbox/ holds hook events queued while the store was busy
  v1/           the old store, if this one was upgraded from vibesnap 1
```

Plain git can read the history: `git --git-dir=.vibe log --stat`.

- **Capture.** Before an agent acts (a new prompt, a shell command), vibesnap records whatever changed since the last event as `external`. The agent's own event then holds only its own change. Edit tools are recorded from the file path in the hook payload, without walking the tree; shell commands use the changed-file list Claude Code provides when it has one, otherwise a scan. Every event carries the session, turn, subagent and tool call that caused it. Prompt text is never stored.
- **Speed.** A stat cache (mtime, ctime, size, inode, mode, blob ID per file, with git's "racy" rule) means unchanged files are never read; a scan is a parallel stat walk that prunes ignored directories. In a git repository the first scan takes blob IDs from `.git/index`, and blobs git already has are borrowed rather than copied. A one-file event writes only the trees on that file's path.
- **Safety.** Objects are written by temp file and rename, then the ledger ref moves by compare-and-swap, then the index commits; if a crash interrupts, the next write catches the index up from the ledger. One writer at a time (a lock on `.vibe/lock`); hooks never wait on it for more than 50 ms, they queue instead. `.vibe`, `.git`, nested repositories, `.env*`, `*.pem`, `*.key` and files over 5 MB are never recorded, and ignored files are never written or deleted.
- **Retention.** `vs gc` keeps 7 days of history by default and everything that a label points at; labels copy any objects they borrow from `.git`, so pruning git cannot break them.
- **Commits.** When a turn ends with uncommitted agent changes, the Stop hook shows you a one-line offer (it costs no model turn). `vs commit turn` stages only the files agents changed in that turn; if you also edited one of them during the turn it asks for `--include-mixed`.

### Configuration

`~/.config/vibesnap/config.toml` (all optional):

```toml
commit_offer = "on"          # "off", or "ask-agent" to have Claude ask you at the end of a turn
retention_days = 7
max_file_size = 5242880      # bytes; larger files are not recorded
auto_init = true             # let the SessionStart hook create a store at a git top level
auto_init_max_files = 100000 # ...but not in bigger repositories
secret_patterns = []         # extra patterns never recorded (gitignore syntax)
```

## Performance

Measured with `bench/bench.py` on a 4-core Linux container: 20,000 text files (134 MiB) plus 50,000 ignored files in `node_modules/`. Spawning `/bin/true` costs about 1 ms on this machine, which is included in every hook number. CI runs the same benchmark on each pull request.

| Operation | vibesnap 2 | vibesnap 1 | git | Roadmap budget |
| --- | --- | --- | --- | --- |
| First snapshot, git repo | 0.19–0.37 s | 2.95 s | 4.6–6.9 s (`add -A`) | 0.1 s |
| First snapshot, no git | 1.7–3.6 s | 2.95 s | — | 3 s |
| Snapshot with nothing changed | 40–56 ms | 1.56 s | 42 ms (`status`) | 50 ms |
| Edit hook (PostToolUse), median / p95 | 7 / 9 ms | — | — | 5 ms |
| Edit hook (PreToolUse), p95 | 4 ms | — | — | 2 ms |
| Prompt hook (reconcile, nothing changed), p95 | 61 ms | — | — | 50 ms |
| Restore, one file differs | 85–110 ms | 2.27 s (rewrites all) | — | proportional to the change |
| Store size, git repo, after gc | 3.9 MB | 181 MB | 3.9 MB (`.git`) | no larger than `.git` |

Ranges are across runs on this (noisy) machine. Where a budget is missed, the remaining time is mostly process start-up and SQLite. Set `VIBESNAP_TRACE=1` to print where a scan or hook spends its time.

## Not done yet

From the roadmap, still open:

- Isolated lanes for subagents (`vs fork`, `vs adopt`, `vs lanes`) and `vs bisect`.
- Hook adapters for Cursor, Codex CLI and Gemini CLI; `vs watch` for agents without hooks; an MCP server.
- `vs blame --format agent-trace`.
- In SHA-256 git repositories nothing is borrowed from `.git` (gitoxide's SHA-256 support is incomplete), so the store holds its own copies.

## Upgrading from vibesnap 1

The first `vs` command in a v1 project moves the old store to `.vibe/v1` and imports every checkpoint as a label `v1-<ID>` (`vs label` lists them). v1 could not tell whether a checkpoint covered the whole tree, so restoring an imported label writes its files but deletes nothing. Tracks, `branch`, `switch` and the interactive pickers are gone; labels and refs replace them.

## Development

```bash
cargo test                                   # unit and end-to-end tests
cargo clippy --all-targets -- -D warnings
python3 bench/bench.py target/release/vs /tmp/vs-bench
```

Start reading at `src/lib.rs`; each module's header comment explains its part (`store/` the ledger and index, `wc/` the working tree, `hook/` capture, `cli/` the commands).

## License

MIT
