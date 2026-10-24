//! Regression tests for the correctness review of vibesnap 2: each test is a
//! reproduction that used to lose data, corrupt the store or misattribute.

mod common;
use common::Project;
use serde_json::{Value, json};
use std::fs;

fn hold_lock(p: &Project) -> fs::File {
    let lock = fs::OpenOptions::new()
        .write(true)
        .open(p.path(".vibe/lock"))
        .unwrap();
    lock.lock().unwrap();
    lock
}

fn pre_edit(p: &Project, rel: &str, id: &str, session: &str) {
    p.hook(
        json!({"hook_event_name": "PreToolUse", "session_id": session, "tool_name": "Write",
        "tool_use_id": id, "tool_input": {"file_path": p.path(rel)}}),
    );
}

fn post_edit(p: &Project, rel: &str, id: &str, session: &str) {
    p.hook(
        json!({"hook_event_name": "PostToolUse", "session_id": session, "tool_name": "Write",
        "tool_use_id": id, "tool_input": {"file_path": p.path(rel)}, "tool_response": {}}),
    );
}

#[test]
fn restore_records_files_with_an_edit_in_flight() {
    let p = Project::git();
    p.write("a.txt", "v1\n");
    let p = p.init();
    p.write("a.txt", "v2\n");
    p.ok(".", &["snap"]);
    pre_edit(&p, "a.txt", "w1", "s1");
    p.write("a.txt", "USER WORK precious\n");
    let out = p.json(".", &["restore", "e:1"]);
    assert_eq!(p.read("a.txt"), "v1\n");
    let saved = out["before"].as_i64().unwrap();
    assert_eq!(
        p.ok(".", &["show", &format!("a.txt@e:{saved}")]),
        "USER WORK precious\n"
    );
}

#[test]
fn undo_never_overwrites_ignored_or_oversized_files() {
    let p = Project::git();
    p.write("notes.txt", "old notes\n");
    let p = p.init();
    fs::remove_file(p.path("notes.txt")).unwrap();
    let deleted = p.json(".", &["snap"])["event"].as_i64().unwrap();
    p.write(".gitignore", "node_modules/\nnotes.txt\n");
    p.write("notes.txt", "new private notes\n");
    p.ok(".", &["snap"]);
    let out = p.run(".", &["undo", &format!("e:{deleted}")]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(p.read("notes.txt"), "new private notes\n");

    // Above the size cap.
    let q = Project::git();
    q.write("big.txt", "small\n");
    let q = q.init();
    fs::remove_file(q.path("big.txt")).unwrap();
    let deleted = q.json(".", &["snap"])["event"].as_i64().unwrap();
    q.write(".config-home/vibesnap/config.toml", "");
    let config = q.path("cfg");
    fs::create_dir_all(config.join("vibesnap")).unwrap();
    fs::write(config.join("vibesnap/config.toml"), "max_file_size = 100\n").unwrap();
    q.write("big.txt", &"x".repeat(680));
    let out = q
        .command(".", &["undo", &format!("e:{deleted}")])
        .env("XDG_CONFIG_HOME", &config)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(q.read("big.txt").len(), 680);
}

#[test]
fn restore_decides_exclusions_before_changing_ignore_files() {
    let p = Project::git();
    p.write("notes.txt", "old notes\n");
    let p = p.init();
    fs::remove_file(p.path("notes.txt")).unwrap();
    p.ok(".", &["snap"]);
    p.write(".gitignore", "node_modules/\nnotes.txt\n");
    p.write("notes.txt", "new private notes\n");
    p.ok(".", &["snap"]);
    let out = p.json(".", &["restore", "e:1"]);
    assert_eq!(p.read("notes.txt"), "new private notes\n", "{out}");
    assert_eq!(p.read(".gitignore"), "node_modules/\n");
}

#[test]
fn git_index_is_not_trusted_when_git_converts_line_endings() {
    let p = Project::git();
    p.git_cmd(&["config", "core.autocrlf", "input"]);
    for i in 0..80 {
        p.write(&format!("f{i}.txt"), "line one\r\nline two\r\n");
    }
    p.git_cmd(&["add", "-A"]);
    p.git_cmd(&["commit", "-q", "-m", "crlf"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    p.git_cmd(&["update-index", "-q", "--refresh"]);
    let p = p.init();
    assert_eq!(
        p.ok(".", &["show", "f1.txt@e:1"]),
        "line one\r\nline two\r\n"
    );
    p.write("f1.txt", "changed\n");
    p.ok(".", &["restore", "e:1", "--", "f1.txt"]);
    assert_eq!(p.read("f1.txt"), "line one\r\nline two\r\n");
}

#[test]
fn after_gc_the_store_does_not_depend_on_git_objects() {
    let p = Project::git().init();
    p.write("keep.txt", "keep me\n");
    p.ok(".", &["label", "keepme"]);
    p.ok(".", &["gc"]);
    // Simulate git pruning everything vibesnap borrowed.
    fs::remove_file(p.path(".vibe/objects/info/alternates")).unwrap();
    fs::remove_file(p.path("src/main.rs")).unwrap();
    p.ok(".", &["restore", "keepme"]);
    assert!(p.read("src/main.rs").contains("hi"));
    p.write("new.txt", "still writable\n");
    assert_eq!(p.json(".", &["snap"])["created"], true);
    p.ok(".", &["doctor"]);
}

#[test]
fn trees_are_never_borrowed_from_git() {
    let p = Project::git().init();
    p.write("src/lib.rs", "x\n");
    p.ok(".", &["snap"]);
    // The baseline tree equals git's HEAD tree, but must be stored in .vibe so
    // the ledger stays readable (and writable) if git prunes its copy.
    fs::remove_file(p.path(".vibe/objects/info/alternates")).unwrap();
    for r in ["refs/vibesnap/ledger", "refs/vibesnap/ledger~1"] {
        let out = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(p.path(".vibe"))
            .args(["ls-tree", "-r", "-t", "--name-only", r])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{r}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn refs_past_the_start_of_history_are_errors() {
    let p = Project::git().init();
    p.write("a.txt", "a\n");
    p.ok(".", &["snap"]);
    for r in ["@-5", "t:-30d", "e:0"] {
        let out = p.run(".", &["restore", r, "--dry-run"]);
        assert_eq!(out.status.code(), Some(2), "{r}");
    }
    assert!(p.exists("a.txt") && p.exists("src/main.rs"));
}

#[test]
fn reindex_after_gc_keeps_the_first_event_a_diff() {
    let p = Project::git().init();
    for f in ["a.txt", "b.txt", "c.txt"] {
        p.write(f, "x\n");
        p.ok(".", &["snap"]);
    }
    p.write("d.txt", "x\n");
    let last = p.json(".", &["snap"])["event"].as_i64().unwrap();
    p.ok(".", &["gc", "--keep", "0m"]);
    p.ok(".", &["reindex"]);
    let e = p.json(".", &["show", &format!("e:{last}")]);
    let files: Vec<&str> = e["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(files, vec!["d.txt"]);
    p.ok(".", &["undo", &format!("e:{last}")]);
    assert!(!p.exists("d.txt") && p.exists("a.txt"));
}

#[test]
fn commit_notices_user_edits_made_after_the_agent() {
    let p = Project::git().init();
    p.prompt("p1");
    p.edit("Edit", "src/main.rs", "t1", None, || {
        p.write("src/main.rs", "fn main() {}\n")
    });
    p.write("src/main.rs", "fn main() {}\n// TOKEN=abc\n");
    let out = p.run(".", &["commit", "-m", "x"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(!p.git_cmd(&["show", "HEAD:src/main.rs"]).contains("TOKEN"));
}

#[test]
fn a_new_prompt_in_one_session_keeps_another_sessions_intents() {
    let p = Project::git().init();
    pre_edit(&p, "a.txt", "w1", "s1");
    p.write("a.txt", "agent work\n");
    p.hook(json!({"hook_event_name": "UserPromptSubmit", "session_id": "s2", "prompt_id": "q1"}));
    post_edit(&p, "a.txt", "w1", "s1");
    let log = p.log(&[]);
    assert_eq!(log[0]["cause"], "write");
    assert_eq!(log[0]["files"][0]["path"], "a.txt");
    assert!(log.iter().all(|e| e["cause"] != "external"));
}

#[test]
fn queued_shell_commands_are_not_blamed_on_the_user() {
    let p = Project::git().init();
    let lock = hold_lock(&p);
    p.hook(json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_use_id": "b1", "tool_input": {"command": "x"}}));
    p.write("gen.txt", "generated\n");
    p.hook(json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_use_id": "b1", "tool_input": {"command": "x"}, "tool_response": {}}));
    drop(lock);
    p.ok(".", &["snap"]);
    let log = p.log(&[]);
    assert_eq!(log[0]["cause"], "bash");
    assert_eq!(log[0]["files"][0]["path"], "gen.txt");
    assert!(log[0]["ambiguous"].is_string());
    assert!(log.iter().all(|e| e["cause"] != "external"));
}

#[test]
fn an_edit_queued_past_stop_stays_in_its_turn() {
    let p = Project::git().init();
    p.prompt("p1");
    pre_edit(&p, "late.txt", "w1", "s1");
    p.write("late.txt", "late\n");
    let lock = hold_lock(&p);
    post_edit(&p, "late.txt", "w1", "s1");
    p.hook(json!({"hook_event_name": "Stop", "prompt_id": "p1"}));
    drop(lock);
    p.ok(".", &["snap"]);
    p.ok(".", &["undo", "turn"]);
    assert!(!p.exists("late.txt"));
}

#[test]
fn undo_keeps_bytes_that_are_not_utf8() {
    let p = Project::git().init();
    fs::write(p.path("menu.txt"), b"caf\xe9\nsoup\n").unwrap();
    p.ok(".", &["snap"]);
    fs::write(p.path("menu.txt"), b"caf\xe9\nsoup\nbread\n").unwrap();
    let add = p.json(".", &["snap"])["event"].as_i64().unwrap();
    fs::write(p.path("menu.txt"), b"tea\ncaf\xe9\nsoup\nbread\n").unwrap();
    p.ok(".", &["snap"]);
    p.ok(".", &["undo", &format!("e:{add}")]);
    assert_eq!(
        fs::read(p.path("menu.txt")).unwrap(),
        b"tea\ncaf\xe9\nsoup\n"
    );
}

#[test]
fn undo_refuses_to_write_through_a_symlink_into_git() {
    let p = Project::git();
    p.write("x/pre-commit", "#!/bin/sh\necho hi\n");
    let p = p.init();
    fs::remove_dir_all(p.path("x")).unwrap();
    let deleted = p.json(".", &["snap"])["event"].as_i64().unwrap();
    std::os::unix::fs::symlink(".git/hooks", p.path("x")).unwrap();
    let out = p.run(".", &["undo", &format!("e:{deleted}")]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!p.exists(".git/hooks/pre-commit"));
}

#[test]
fn paths_through_a_symlinked_directory_are_not_redirected() {
    let p = Project::git();
    p.write("b/x.txt", "b\n");
    let p = p.init();
    std::os::unix::fs::symlink("b", p.path("a")).unwrap();
    p.write("b/x.txt", "changed\n");
    let out = p.run(".", &["restore", "e:1", "--", "a/x.txt"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(p.read("b/x.txt"), "changed\n");
    let _: Value = Value::Null;
}
