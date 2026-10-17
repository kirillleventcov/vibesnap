# VibeSnap

Snapshot-style version control for AI-first coding.

VibeSnap is a lightweight checkpointing tool. It saves and restores file snapshots without staging, indexing, or ceremony. Think of it as git commits without the git.

## Installation

```bash
cargo install vibesnap
```

Or build from source:

```bash
git clone https://github.com/kirillleventcov/vibesnap
cd vibesnap
cargo build --release
```

The binary is at `./target/release/vibesnap`.

## Quick Start

```bash
# Initialize a repo
vibesnap init

# Save a checkpoint
vibesnap snap --note "initial setup"

# List checkpoints
vibesnap list

# Restore a previous checkpoint
vibesnap restore <id>
```

## How It Works

VibeSnap stores data in `.vibe/` at your project root:

```
.vibe/
├── objects/      # File contents stored by SHA-256 hash (deduplicated)
├── snapshots/    # JSON manifests mapping file paths to object hashes
├── meta.db       # SQLite database: checkpoints, tracks and HEAD
└── lock          # Held by any command that writes, so parallel runs queue up
```

When you `snap`, files are hashed and stored in `objects/`. A manifest in `snapshots/` records the state of the tree. When you `restore`, the current state is saved as a checkpoint first, then only the files that differ are written and files the checkpoint does not have are deleted.

`.vibe/` and `.git/` are never snapshotted or restored into, whatever your ignore files say. Files matched by `.gitignore` (or `.vibeignore` when there is no `.gitignore`) are never read, written or deleted.

## CLI Reference

### Repository Setup

- `vibesnap init [path]`
  Initialize a new VibeSnap repo. Defaults to the current directory.

- `vibesnap reset [--confirm]`
  Delete the `.vibe/` directory and all checkpoints. Asks for confirmation on a terminal; without one, `--confirm` is required.

### Core Workflow

- `vibesnap snap [paths...] [--note <message>] [--file <path>...]`
  Create a checkpoint. Does nothing if nothing changed since HEAD.
  - `[paths...]`: Files or directories to re-read, relative to the current directory. The rest of the tree keeps its state from HEAD, so the checkpoint still describes the whole tree. Defaults to the whole repo.
  - `--note`: Optional description.
  - `--file`: Same as passing paths. Repeatable.

- `vibesnap list [--track <name>] [-i] [--file <path>]`
  List checkpoints in compact timeline format.
  - `--track`: Filter to a specific track.
  - `--interactive`: Select a checkpoint from a menu.
  - `--file`: Show only checkpoints containing the given file.

- `vibesnap restore <id> [-i] [--file <path>...]`
  Bring the working tree to a checkpoint's state. The current state is saved as a new checkpoint first, and its ID is printed, so a restore can be undone by restoring that ID.
  - `<id>`: Checkpoint ID (case-insensitive).
  - `-i, --interactive`: Select checkpoint from a menu (needs a terminal).
  - `--file`: Restore only these files or directories, relative to the current directory. Repeatable.
  - Checkpoints made by older versions of vibesnap, and partial snaps taken before any full one, may not describe the whole tree; restoring them writes their files but deletes nothing.

- `vibesnap diff <id1> <id2> [path] [-i]`
  Show unified diff between two checkpoints.
  - `<id1> <id2>`: Checkpoint IDs, older first.
  - `[path]`: Diff only this file or directory, relative to the current directory.
  - `-i, --interactive`: Select both checkpoints from a menu (needs a terminal).

### Branching

- `vibesnap branch <name> [--from-id <id>]`
  Create a new track. Defaults to branching from the current HEAD.

- `vibesnap switch <name> [-i]`
  Switch to a track and restore its latest checkpoint, saving the current state first.
  - `<name>`: Track name.
  - `-i, --interactive`: Select track from a menu (needs a terminal).

### Configuration

- `vibesnap config show`
  Display the current configuration.

- `vibesnap config path`
  Show the configuration file location.

### Scripting and agents

- Nothing prompts unless you pass `-i`. A missing argument is an error that lists the valid choices.
- `--json` works on every command and prints one JSON object with `"v": 1`. Errors go to stderr as `{"v":1,"error":{"kind":...,"message":...}}`.
- Exit codes: `0` success (including "nothing changed"), `1` error, `2` usage error.
- Commands that write wait for each other through `.vibe/lock`, so running several at once is safe.

## Example Workflow

```bash
# Start a project
$ vibesnap init
Initialised empty VibeSnap repo in /path/to/project/.vibe

# Save progress
$ vibesnap snap --note "baseline"
snap A1B2C3D4 (15 files) - baseline

# Make changes, save again
$ vibesnap snap src/main.rs --note "refactor main"
snap E5F6G7H8 (15 files; re-read: src/main.rs) - refactor main

# See your history
$ vibesnap list
◆ 2025-09-27 10:30:00 A1B2C3D4 - baseline
● 2025-09-27 10:35:15 E5F6G7H8 - refactor main

# Oops, go back
$ vibesnap restore A1B2C3D4
Restored checkpoint A1B2C3D4: 1 written, 0 deleted, 14 unchanged

# Try something experimental on a new track
$ vibesnap branch experiment --from-id A1B2C3D4
Created track experiment at A1B2C3D4

$ vibesnap switch experiment
Restored checkpoint A1B2C3D4: 0 written, 0 deleted, 15 unchanged
Switched to track experiment

# Snap on the new track
$ vibesnap snap --note "wild idea"
snap I9J0K1L2 (15 files) - wild idea

# Back to main
$ vibesnap switch main
Restored checkpoint E5F6G7H8: 2 written, 0 deleted, 13 unchanged
Switched to track main
```

## Comparison to Git

| Feature            | Git                                | VibeSnap                          |
| ------------------ | ---------------------------------- | --------------------------------- |
| Core unit          | Commit                             | Checkpoint                        |
| Staging area       | Yes (the index)                    | No                                |
| Message required   | Required                           | Optional                          |
| Navigation         | checkout, reset, revert            | restore `<id>`                    |
| Workflow           | Edit -> Stage -> Commit            | Edit -> Snap                      |
| Branching          | Branches                           | Tracks                            |
| Use case           | Collaborative development          | Rapid personal iteration          |
