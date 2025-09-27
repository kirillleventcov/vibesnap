# VibeSnap

Snapshot-style version control for AI-first coding.

VibeSnap is a command-line tool designed for a fast, iterative workflow, particularly suited for projects involving AI-assisted coding. It focuses on simplicity and speed, allowing you to save and restore "checkpoints" of your work without the complexity of traditional version control systems.

## Core Concepts

| Concept        | Description                                                                 |
| -------------- | --------------------------------------------------------------------------- |
| **Checkpoint** | An immutable snapshot of specified files or directories at a point in time. |
| **Track**      | An ordered sequence of checkpoints, similar to a branch in Git.             |
| **HEAD**       | A pointer to the current track and the most recent checkpoint restored.     |
| **.vibe/**     | A hidden directory at the repository root for storing all VibeSnap data.    |

## Installation

### Prerequisites

- Rust 1.85+ (2024) (install from [https://rustup.rs/](https://rustup.rs/)):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Build from Source

1.  Clone the repository:

    ```bash
    git clone https://github.com/your-username/VibeSnap
    cd VibeSnap
    ```

2.  Build the release binary:

    ```bash
    cargo build --release
    ```

    The binary will be located at `./target/release/vibesnap`.

3.  (Optional) Install for system-wide access:
    ```bash
    cargo install --path .
    ```
    You can now use `vibesnap` from any directory.

## CLI Reference

### Repository Management

- `vibesnap init [path]`
  Initializes a new VibeSnap repository in the specified directory (or the current one if omitted).

- `vibesnap reset [--confirm]`
  Deletes the `.vibe` repository and all stored checkpoints.

### Core Workflow

- `vibesnap snap [paths...] [--note "message"] [--progress] [--files <files...>] [--file <file>]`
  Creates a new checkpoint.
  - `[paths...]`: One or more paths to include (defaults to current directory).
  - `--note`: Attach a descriptive note.
  - `--progress`: Show a progress bar, useful for large projects.
  - `--files`: Specify a comma-separated list of files to snap.
  - `--file`: Specify a single file to snap.

- `vibesnap list [--track <name>] [--tree] [--interactive] [--file <file>]`
  Lists existing checkpoints.
  - `--track`: Filter by a specific track.
  - `--tree`: Display the file tree for each checkpoint.
  - `--interactive`: Enter an interactive mode to browse and select checkpoints.
  - `--file`: Show only checkpoints containing the specified file.

- `vibesnap restore [id] [--interactive] [--progress] [--files <files...>] [--file <file>] [--interactive-files]`
  Restores the working directory to a previous state.
  - `[id]`: The ID of the checkpoint to restore. If omitted, enters interactive mode.
  - `--interactive`: Use fuzzy search to select a checkpoint to restore.
  - `--progress`: Show a progress bar during the restore process.
  - `--files`: Restore only a comma-separated list of files from the checkpoint.
  - `--file`: Restore only a single specified file.
  - `--interactive-files`: Interactively select which files to restore from the checkpoint.

- `vibesnap latest [--progress]`
  Restores the most recent checkpoint on the current track.

### Branching and Tracks

- `vibesnap branch <name> [--from-id <id>]`
  Creates a new track.
  - `<name>`: The name for the new track.
  - `--from-id`: The checkpoint ID to branch from (defaults to the current HEAD).

- `vibesnap switch [<name>] [--interactive]`
  Switches to a different track.
  - `<name>`: The name of the track to switch to. If omitted, enters interactive mode.
  - `--interactive`: Use fuzzy search to select a track.

### Inspection and History

- `vibesnap diff [<id1>] [<id2>] [--file <path>] [--side-by-side] [--interactive]`
  Shows the difference between two checkpoints.
  - `[<id1>] [<id2>]`: The two checkpoint IDs to compare. If omitted, enters interactive mode.
  - `--file`: Limit the diff to a specific file.
  - `--side-by-side`: Display the diff in a side-by-side format.
  - `--interactive`: Interactively select the two checkpoints to compare.

- `vibesnap graph [--track <name>] [--detailed] [--compact]`
  Displays a visual graph of checkpoints and tracks.
  - `--track`: Show only a specific track in the graph.
  - `--detailed`: Show more detailed information for each checkpoint.
  - `--compact`: Use a more compact layout.

### Configuration

- `vibesnap config <subcommand>`
  Manages configuration settings.
  - `show`: Display the current configuration.
  - `edit`: Open the configuration file in the default editor.
  - `set <key> <value>`: Set a configuration value.
  - `get <key>`: Get a configuration value.
  - `reset`: Reset the configuration to its default state.
  - `path`: Show the location of the configuration file.

### Interactive Shortcuts

- `vibesnap select <action>`
  Provides direct access to interactive modes.
  - `restore`: Interactive checkpoint restore.
  - `switch`: Interactive track switching.
  - `diff`: Interactive diff comparison.

## Example Workflow

1.  **Initialize a repository**

    ```bash
    $ vibesnap init
    Initialised empty VibeSnap repo in /path/to/project/.vibe
    ```

2.  **Create checkpoints**
    Make some changes to your files.

    ```bash
    $ vibesnap snap --note "Initial implementation"
    ✓ snap A1B2C3D4 (15 files) - Initial implementation

    $ # Make more changes
    $ vibesnap snap src/main.rs --note "Refactor main function"
    ✓ snap E5F6G7H8 (1 file: src/main.rs) - Refactor main function
    ```

3.  **List checkpoints**

    ```bash
    $ vibesnap list
    ┌──────────┬───────┬──────────┬─────────────────────┬──────────────────────────┐
    │ id       │ track │ parent   │ when                │ note                     │
    ├──────────┼───────┼──────────┼─────────────────────┼──────────────────────────┤
    │ A1B2C3D4 │ main  │ -        │ 2025-09-27 10:30:00 │ Initial implementation   │
    │ E5F6G7H8 │ main  │ A1B2C3D4 │ 2025-09-27 10:35:15 │ Refactor main function   │
    └──────────┴───────┴──────────┴─────────────────────┴──────────────────────────┘
    ```

4.  **Restore a previous state**

    ```bash
    $ vibesnap restore A1B2C3D4
    Workspace restored to A1B2C3D4
    ```

5.  **Create a new track to experiment**

    ```bash
    $ vibesnap branch feature-x --from-id A1B2C3D4
    Created track feature-x at A1B2C3D4

    $ vibesnap switch feature-x
    Switched to track feature-x and restored checkpoint A1B2C3D4
    ```

6.  **Work on the new track**
    Any new snaps will be on the `feature-x` track.

    ```bash
    $ # ... make changes ...
    $ vibesnap snap --note "Add experimental feature"
    ✓ snap I9J0K1L2 (20 files) - Add experimental feature
    ```

7.  **Switch back to the main track**
    ```bash
    $ vibesnap switch main
    Switched to track main and restored checkpoint E5F6G7H8
    ```
    Your working directory is now back to the latest state of the `main` track.

## How It Works

VibeSnap stores all its data in the `.vibe` directory at the root of your project. It uses a content-addressed storage model for efficiency.

```
.vibe/
├── objects/      # Stores unique file contents, named by their SHA-256 hash.
├── snapshots/    # Contains JSON files (manifests) for each checkpoint, mapping file paths to content hashes.
├── meta.db       # An SQLite database tracking checkpoints, tracks, and metadata.
└── HEAD          # A simple text file indicating the current track and restored checkpoint.
```

- **Snapping:** When you `snap`, VibeSnap hashes the content of each file. If the hash is new, the content is stored in `objects/`. A manifest is created in `snapshots/` that lists all file paths and their corresponding hashes for that checkpoint.
- **Restoring:** When you `restore`, VibeSnap reads the manifest for the specified checkpoint and copies the corresponding files from the `objects/` directory back into your working tree.
- **Efficiency:** Because files are stored by their content hash, duplicate files (even with different names or across different checkpoints) are stored only once.

## Comparison to Git

| Feature                 | Git                                | VibeSnap                               |
| ----------------------- | ---------------------------------- | -------------------------------------- |
| **Core Unit**           | Commit                             | Checkpoint                             |
| **Staging Area**        | Yes (the index)                    | No                                     |
| **Message Requirement** | Required for commits               | Optional note                          |
| **Navigation**          | `checkout`, `reset`, `revert`      | `restore <id>`, `latest`               |
| **Workflow**            | Edit -> Stage -> Commit            | Edit -> Snap                           |
| **Splitting Work**      | Worktree                           | Branches                               |
| **Target Use Case**     | Collaborative software development | Rapid, individual, iterative workflows |
