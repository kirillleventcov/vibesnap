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
├── meta.db       # SQLite database with checkpoint and track metadata
└── HEAD          # Current track and checkpoint pointer
```

When you `snap`, files are hashed and stored in `objects/`. A manifest in `snapshots/` records what files were included. When you `restore`, files are copied from `objects/` back into your working tree.

## CLI Reference

### Repository Setup

- `vibesnap init [path]`
  Initialize a new VibeSnap repo. Defaults to the current directory.

- `vibesnap reset [--confirm]`
  Delete the `.vibe/` directory and all checkpoints. Requires confirmation unless `--confirm` is passed.

### Core Workflow

- `vibesnap snap [paths...] [--note <message>] [--file <path>...]`
  Create a checkpoint.
  - `[paths...]`: Directories or files to snapshot. Defaults to the current directory.
  - `--note`: Optional description.
  - `--file`: Snapshot only specific files. Repeatable.

- `vibesnap list [--track <name>] [--interactive] [--file <path>]`
  List checkpoints in compact timeline format.
  - `--track`: Filter to a specific track.
  - `--interactive`: Select a checkpoint from a menu.
  - `--file`: Show only checkpoints containing the given file.

- `vibesnap restore [id] [--interactive] [--file <path>...]`
  Restore files from a checkpoint to the working tree.
  - `[id]`: Checkpoint ID. Omit to enter interactive selection.
  - `--interactive`: Select checkpoint from a menu.
  - `--file`: Restore only specific files. Repeatable.

- `vibesnap diff [id1] [id2] [--file <path>] [--interactive]`
  Show unified diff between two checkpoints.
  - `[id1] [id2]`: Checkpoint IDs. Omit either to enter interactive selection.
  - `--file`: Diff only the specified file.
  - `--interactive`: Select both checkpoints from a menu.

### Branching

- `vibesnap branch <name> [--from-id <id>]`
  Create a new track. Defaults to branching from the current HEAD.

- `vibesnap switch [<name>] [--interactive]`
  Switch to a track and restore its latest checkpoint.
  - `[name]`: Track name. Omit to enter interactive selection.
  - `--interactive`: Select track from a menu.

### Configuration

- `vibesnap config show`
  Display the current configuration.

- `vibesnap config path`
  Show the configuration file location.

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
snap E5F6G7H8 (1 file: src/main.rs) - refactor main

# See your history
$ vibesnap list
◆ 2025-09-27 10:30:00 A1B2C3D4 - baseline
● 2025-09-27 10:35:15 E5F6G7H8 - refactor main

# Oops, go back
$ vibesnap restore A1B2C3D4
Restored (15 files) from checkpoint A1B2C3D4

# Try something experimental on a new track
$ vibesnap branch experiment --from-id A1B2C3D4
Created track experiment at A1B2C3D4

$ vibesnap switch experiment
Switched to track experiment and restored checkpoint A1B2C3D4

# Snap on the new track
$ vibesnap snap --note "wild idea"
snap I9J0K1L2 (15 files) - wild idea

# Back to main
$ vibesnap switch main
Switched to track main and restored checkpoint A1B2C3D4
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
