//! P1: the store, the stat cache, restore, gc, reindex and migration.

mod common;
use common::Project;
use serde_json::Value;
use std::fs;
use std::process::Stdio;

fn paths(event: &Value) -> Vec<String> {
    event["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn init_records_a_baseline_borrowing_from_git() {
    let p = Project::git();
    p.write(".env", "SECRET=1\n");
    p.write("keys/server.pem", "-----BEGIN-----\n");
    p.write("node_modules/x/index.js", "ignored\n");
    let out = p.json(".", &["init"]);
    assert_eq!(out["created"], true);
    assert_eq!(out["borrowing_from_git"], true);
    assert_eq!(out["files"], 2, "{out}");
    let log = p.log(&[]);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["cause"], "baseline");
    assert_eq!(paths(&log[0]), vec![".gitignore", "src/main.rs"]);
    assert!(
        fs::read_to_string(p.path(".git/info/exclude"))
            .unwrap()
            .contains(".vibe/")
    );
    // Blobs git already has are borrowed, not copied.
    let loose_blobs = fs::read_dir(p.path(".vibe/objects"))
        .unwrap()
        .filter_map(|d| d.ok())
        .filter(|d| d.file_name().len() == 2)
        .flat_map(|d| fs::read_dir(d.path()).unwrap())
        .count();
    assert!(
        loose_blobs <= 3,
        "only trees and the commit are stored, found {loose_blobs}"
    );
    assert_eq!(p.ledger_length(), 1);
    // Init again is a no-op.
    assert_eq!(p.json(".", &["init"])["created"], false);
}

#[test]
fn works_outside_git_and_never_records_the_store() {
    let p = Project::bare();
    p.write("a.txt", "1\n");
    let p = p.init();
    p.write("a.txt", "2\n");
    p.ok(".", &["snap"]);
    for event in p.log(&[]) {
        assert!(
            paths(&event).iter().all(|f| !f.starts_with(".vibe")),
            "{event}"
        );
    }
}

#[test]
fn snap_with_nothing_changed_records_nothing() {
    let p = Project::git().init();
    let out = p.json(".", &["snap"]);
    assert_eq!(out["created"], false);
    assert_eq!(p.log(&[]).len(), 1);
}

#[test]
fn stat_cache_catches_same_size_rewrites_and_mode_changes() {
    use std::os::unix::fs::PermissionsExt;
    let p = Project::git().init();
    // Same size, written in the same instant as the baseline scan: the racy
    // rule must still notice it.
    p.write("src/main.rs", "fn main() {\n    println!(\"ho\");\n}\n");
    fs::set_permissions(p.path(".gitignore"), fs::Permissions::from_mode(0o755)).unwrap();
    let status = p.json(".", &["status"]);
    let changed: Vec<&str> = status["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert_eq!(changed, vec![".gitignore", "src/main.rs"]);
}

#[test]
fn restore_saves_state_deletes_newer_files_and_keeps_ignored_ones() {
    let p = Project::git().init();
    p.write("same.txt", "same\n");
    p.ok(".", &["snap", "--label", "base"]);
    p.write("src/main.rs", "changed\n");
    p.write("src/new.rs", "new\n");
    p.write("node_modules/dep.js", "ignored\n");
    let before = fs::metadata(p.path("same.txt"))
        .unwrap()
        .modified()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    let out = p.json(".", &["restore", "base"]);
    assert_eq!(out["written"], serde_json::json!(["src/main.rs"]));
    assert_eq!(out["deleted"], serde_json::json!(["src/new.rs"]));
    assert!(p.read("src/main.rs").contains("hi"));
    assert!(!p.exists("src/new.rs"));
    assert_eq!(p.read("node_modules/dep.js"), "ignored\n");
    assert_eq!(
        fs::metadata(p.path("same.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );

    // The state before the restore was recorded, so it can be undone.
    let back = out["before"].as_i64().unwrap();
    p.ok(".", &["restore", &format!("e:{back}")]);
    assert_eq!(p.read("src/main.rs"), "changed\n");
    assert_eq!(p.read("src/new.rs"), "new\n");
}

#[test]
fn restore_keeps_exec_bits_and_symlinks() {
    use std::os::unix::fs::PermissionsExt;
    let p = Project::git();
    p.write("run.sh", "echo 1\n");
    fs::set_permissions(p.path("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("src/main.rs", p.path("link")).unwrap();
    let p = p.init();
    fs::remove_file(p.path("run.sh")).unwrap();
    fs::remove_file(p.path("link")).unwrap();
    p.ok(".", &["restore", "e:1"]);
    assert_eq!(
        fs::metadata(p.path("run.sh")).unwrap().permissions().mode() & 0o111,
        0o111
    );
    assert_eq!(
        fs::read_link(p.path("link")).unwrap().to_str(),
        Some("src/main.rs")
    );
}

#[test]
fn restore_of_some_paths_leaves_the_rest() {
    let p = Project::git().init();
    p.write("src/main.rs", "changed\n");
    p.write("other.txt", "keep\n");
    p.ok("src", &["restore", "e:1", "--", "main.rs"]);
    assert!(p.read("src/main.rs").contains("hi"));
    assert_eq!(p.read("other.txt"), "keep\n");
}

#[test]
fn parallel_writers_keep_one_chain() {
    const N: usize = 8;
    let p = Project::git().init();
    for i in 0..N {
        p.write(&format!("f{i}.txt"), &format!("{i}\n"));
    }
    let children: Vec<_> = (0..N)
        .map(|i| {
            let payload = serde_json::json!({
                "hook_event_name": "PostToolUse", "session_id": "s", "cwd": p.root(),
                "tool_name": "Write", "tool_use_id": format!("w{i}"),
                "tool_input": {"file_path": p.path(&format!("f{i}.txt"))},
            });
            let mut child = p
                .command(".", &["hook", "claude"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.to_string().as_bytes())
                .unwrap();
            child
        })
        .collect();
    for mut c in children {
        assert!(c.wait().unwrap().success());
    }
    // Some may have been queued if the lock was busy; any write drains the queue.
    p.ok(".", &["snap"]);
    let log = p.log(&[]);
    let writes: Vec<&Value> = log.iter().filter(|e| e["cause"] == "write").collect();
    assert_eq!(writes.len(), N, "{log:?}");
    let seqs: Vec<i64> = log.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
    let expected: Vec<i64> = (1..=seqs.len() as i64).rev().collect();
    assert_eq!(seqs, expected, "sequence numbers are dense and ordered");
    assert_eq!(p.ledger_length(), log.len());
    p.ok(".", &["doctor"]);
}

#[test]
fn hooks_queue_while_the_lock_is_held() {
    let p = Project::git().init();
    let lock = fs::OpenOptions::new()
        .write(true)
        .open(p.path(".vibe/lock"))
        .unwrap();
    lock.lock().unwrap();
    p.write("q.txt", "queued\n");
    let started = std::time::Instant::now();
    p.hook(serde_json::json!({
        "hook_event_name": "PostToolUse", "tool_name": "Write", "tool_use_id": "w1",
        "tool_input": {"file_path": p.path("q.txt")}, "agent_id": "a1", "agent_type": "writer",
    }));
    assert!(
        started.elapsed().as_secs() < 2,
        "the hook did not wait for the lock"
    );
    assert_eq!(fs::read_dir(p.path(".vibe/inbox")).unwrap().count(), 1);
    assert_eq!(p.json(".", &["status"])["pending_hook_events"], 1);
    drop(lock);
    p.ok(".", &["snap"]);
    // The queued write is applied first; the snap then finds nothing left.
    let log = p.log(&[]);
    assert_eq!(log.len(), 2);
    assert_eq!(log[0]["cause"], "write");
    assert_eq!(log[0]["who"], "writer");
    assert_eq!(fs::read_dir(p.path(".vibe/inbox")).unwrap().count(), 0);
}

#[test]
fn gc_prunes_history_but_keeps_labels() {
    let p = Project::git().init();
    p.ok(".", &["label", "start"]);
    for i in 0..3 {
        p.write("a.txt", &format!("{i}\n"));
        p.ok(".", &["snap"]);
    }
    let out = p.json(".", &["gc", "--keep", "0m"]);
    assert_eq!(out["pruned_events"], 3);
    assert_eq!(p.log(&[]).len(), 1);
    assert_eq!(p.ledger_length(), 1);
    p.ok(".", &["doctor"]);
    p.ok(".", &["restore", "start"]);
    assert!(!p.exists("a.txt"));
    // Numbers are never reused after pruning.
    p.write("b.txt", "b\n");
    let snap = p.json(".", &["snap"]);
    assert!(snap["event"].as_i64().unwrap() > 4);
}

#[test]
fn the_index_is_rebuilt_from_the_ledger() {
    let p = Project::git().init();
    p.write("a.txt", "1\n");
    p.ok(".", &["snap", "-m", "one"]);
    let before = p.log(&[]);
    for f in ["index.db", "index.db-wal", "index.db-shm"] {
        let _ = fs::remove_file(p.path(&format!(".vibe/{f}")));
    }
    p.ok(".", &["reindex"]);
    let after = p.log(&[]);
    assert_eq!(after.len(), before.len());
    assert_eq!(after[0]["note"], "one");
    assert_eq!(paths(&after[0]), vec!["a.txt"]);
    // The rebuilt stat cache does not invent changes.
    assert_eq!(p.json(".", &["status"])["changes"], serde_json::json!([]));
    p.write("a.txt", "2\n");
    assert_eq!(p.json(".", &["snap"])["created"], true);
}

#[test]
fn a_v1_store_is_migrated_and_imported_as_partial_labels() {
    let p = Project::bare();
    p.write("a.txt", "old\n");
    p.write("extra.txt", "created later\n");
    let hash = "8b6b14fdb84b77e2b7b9a2dd8bcd3e3e1d7b3d5e3c3a6f0b8b0b7d8c0e7c1a1d";
    p.write(&format!(".vibe/objects/{hash}"), "old\n");
    p.write(
        ".vibe/snapshots/0A0B0C0D.json",
        &format!("{{\"files\": {{\"a.txt\": \"{hash}\", \".vibe/meta.db\": \"{hash}\"}}}}"),
    );
    let conn = rusqlite::Connection::open(p.path(".vibe/meta.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE tracks (name TEXT PRIMARY KEY, head TEXT);
         CREATE TABLE checkpoints (id TEXT PRIMARY KEY, track TEXT, parent TEXT, timestamp INTEGER, note TEXT, is_auto INTEGER DEFAULT 0);
         INSERT INTO checkpoints VALUES ('0A0B0C0D', 'main', NULL, 1700000000, 'old', 0);",
    )
    .unwrap();
    drop(conn);

    let labels = p.json(".", &["label"]);
    assert_eq!(labels["labels"][0]["name"], "v1-0A0B0C0D");
    assert_eq!(labels["labels"][0]["partial"], true);
    assert!(p.exists(".vibe/v1/meta.db"));
    p.write("a.txt", "new\n");
    let out = p.json(".", &["restore", "v1-0A0B0C0D"]);
    assert_eq!(out["deletion_enabled"], false);
    assert_eq!(p.read("a.txt"), "old\n");
    assert_eq!(p.read("extra.txt"), "created later\n");
}

#[test]
fn errors_have_kinds_and_exit_codes() {
    let p = Project::git().init();
    let out = p.run(".", &["restore", "nope", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["error"]["kind"], "usage");
    let out = p.run(".", &["reset"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(p.exists(".vibe"));
    let outside = Project::bare();
    assert_eq!(outside.run(".", &["log"]).status.code(), Some(1));
}
