//! P2: capture through Claude Code hooks.

mod common;
use common::Project;
use serde_json::{Value, json};

#[test]
fn session_start_creates_a_store_at_the_git_top_level() {
    let p = Project::git();
    p.write("deep/dir/x.txt", "x\n");
    let mut payload =
        json!({"hook_event_name": "SessionStart", "source": "startup", "session_id": "s1"});
    payload["cwd"] = p.path("deep/dir").to_string_lossy().to_string().into();
    let mut child = p
        .command(".", &["hook", "claude"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert!(p.exists(".vibe/version"));
    assert!(!p.exists("deep/dir/.vibe"));
}

#[test]
fn hooks_do_nothing_outside_a_project() {
    let p = Project::bare();
    p.write("a.txt", "a\n");
    let out = p.hook(json!({"hook_event_name": "SessionStart", "source": "startup"}));
    assert_eq!(out, "");
    assert!(!p.exists(".vibe"));
}

#[test]
fn every_change_is_attributed() {
    let p = Project::git().init();
    p.hook(json!({"hook_event_name": "SessionStart", "source": "startup"}));
    p.write("notes.txt", "user wrote this before the prompt\n");
    p.prompt("p1");
    p.edit("Edit", "src/main.rs", "t1", None, || {
        p.write("src/main.rs", "fn main() {}\n")
    });
    p.hook(json!({"hook_event_name": "SubagentStart", "prompt_id": "p1", "agent_id": "a1", "agent_type": "test-writer"}));
    p.edit(
        "Write",
        "tests/t.rs",
        "t2",
        Some(("a1", "test-writer")),
        || p.write("tests/t.rs", "#[test]\nfn t() {}\n"),
    );
    p.hook(json!({"hook_event_name": "SubagentStop", "prompt_id": "p1", "agent_id": "a1", "agent_type": "test-writer",
        "last_assistant_message": "Added tests/t.rs with one test."}));
    p.hook(json!({"hook_event_name": "PreToolUse", "prompt_id": "p1", "tool_name": "Bash", "tool_use_id": "t3", "tool_input": {"command": "x"}}));
    p.write("gen.txt", "generated\n");
    p.hook(json!({"hook_event_name": "PostToolUse", "prompt_id": "p1", "tool_name": "Bash", "tool_use_id": "t3",
        "tool_input": {"command": "x"}, "tool_response": {"stdout": ""}}));
    let stop =
        p.hook(json!({"hook_event_name": "Stop", "prompt_id": "p1", "stop_hook_active": false}));

    let log = p.log(&[]);
    let who: Vec<(&str, &str)> = log
        .iter()
        .map(|e| (e["cause"].as_str().unwrap(), e["who"].as_str().unwrap()))
        .collect();
    assert_eq!(
        who,
        vec![
            ("bash", "main"),
            ("write", "test-writer"),
            ("edit", "main"),
            ("external", "external"),
            ("baseline", "user")
        ]
    );
    assert_eq!(log[0]["turn"], 1);
    assert_eq!(log[2]["tool_use_id"], "t1");
    assert_eq!(
        log[3]["turn"],
        Value::Null,
        "the user's edit came before the turn"
    );

    let stop: Value = serde_json::from_str(&stop).unwrap();
    let offer = stop["systemMessage"].as_str().unwrap();
    assert!(
        offer.contains("turn 1 changed 3 files") && offer.contains("main, test-writer"),
        "{offer}"
    );
    // Offered once per turn.
    assert_eq!(
        p.hook(json!({"hook_event_name": "Stop", "prompt_id": "p1"})),
        ""
    );

    let recap = p.ok(".", &["recap"]);
    assert!(
        recap.contains("turn:1") && recap.contains("test-writer: Added tests/t.rs"),
        "{recap}"
    );
    let compact = p.hook(json!({"hook_event_name": "SessionStart", "source": "compact"}));
    let compact: Value = serde_json::from_str(&compact).unwrap();
    assert!(
        compact["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("turn:1")
    );
}

#[test]
fn a_change_made_just_before_an_edit_is_not_blamed_on_the_agent() {
    let p = Project::git().init();
    p.prompt("p1");
    // The user changes the file, then the agent edits it before any reconcile.
    p.write("src/main.rs", "user line\n");
    p.edit("Edit", "src/main.rs", "t1", None, || {
        p.write("src/main.rs", "user line\nagent line\n")
    });
    let log = p.log(&[]);
    assert_eq!(log[0]["cause"], "edit");
    assert_eq!(log[0]["files"][0]["add"], 1);
    assert_eq!(log[0]["files"][0]["del"], 0);
    assert_eq!(log[1]["cause"], "external");
    // Undoing the agent's edit keeps the user's change.
    p.ok(".", &["undo", "@-1"]);
    assert_eq!(p.read("src/main.rs"), "user line\n");
}

#[test]
fn a_scan_skips_files_with_an_edit_in_flight() {
    let p = Project::git().init();
    p.prompt("p1");
    let file = p.path("src/main.rs").to_string_lossy().to_string();
    p.hook(json!({"hook_event_name": "PreToolUse", "tool_name": "Edit", "tool_use_id": "e1", "agent_id": "a1",
        "agent_type": "writer", "tool_input": {"file_path": file}}));
    p.write("src/main.rs", "agent edit\n");
    // Meanwhile another agent runs a command, which reconciles first.
    p.hook(json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_use_id": "b1", "tool_input": {"command": "ls"}}));
    p.hook(json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_use_id": "b1", "tool_input": {"command": "ls"}, "tool_response": {}}));
    assert_eq!(p.log(&[]).len(), 1, "nothing recorded yet");
    p.hook(json!({"hook_event_name": "PostToolUse", "tool_name": "Edit", "tool_use_id": "e1", "agent_id": "a1",
        "agent_type": "writer", "tool_input": {"file_path": file}, "tool_response": {}}));
    let log = p.log(&[]);
    assert_eq!(log[0]["who"], "writer");
    assert_eq!(log.len(), 2);
}

#[test]
fn bash_edit_lists_are_used_and_shared_runs_are_ambiguous() {
    let p = Project::git().init();
    p.prompt("p1");
    p.write("listed.txt", "a\n");
    p.write("unlisted.txt", "b\n");
    p.hook(json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_use_id": "b1", "tool_input": {"command": "x"},
        "tool_response": {"stdout": "", "bashEditDiff": {"changedFiles": [p.path("listed.txt")], "shared": true}}}));
    let log = p.log(&[]);
    assert_eq!(log[0]["files"][0]["path"], "listed.txt");
    assert_eq!(log[0]["files"].as_array().unwrap().len(), 1);
    assert!(log[0]["ambiguous"].is_string());
    // Undo leaves ambiguous events alone unless asked, and never touches
    // changes it did not attribute to an agent (unlisted.txt).
    let out = p.run(".", &["undo", "turn"]);
    assert_eq!(out.status.code(), Some(2));
    p.write("unlisted.txt", "b, edited by the user\n");
    p.ok(".", &["undo", "turn", "--include-ambiguous"]);
    assert!(!p.exists("listed.txt"));
    assert_eq!(p.read("unlisted.txt"), "b, edited by the user\n");
}

#[test]
fn secrets_and_ignored_files_are_never_recorded_by_hooks() {
    let p = Project::git().init();
    p.prompt("p1");
    p.edit("Write", ".env", "t1", None, || p.write(".env", "TOKEN=x\n"));
    p.edit("Write", "node_modules/a.js", "t2", None, || {
        p.write("node_modules/a.js", "x\n")
    });
    assert_eq!(p.log(&[]).len(), 1);
}

#[test]
fn undo_is_selective_and_refuses_conflicts() {
    let p = Project::git();
    p.write("a.txt", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
    p.git_cmd(&["add", "-A"]);
    p.git_cmd(&["commit", "-q", "-m", "a"]);
    let p = p.init();
    p.prompt("p1");
    p.edit("Edit", "a.txt", "t1", Some(("a1", "fixer")), || {
        p.write("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\nseven\n")
    });
    p.edit("Edit", "a.txt", "t2", None, || {
        p.write("a.txt", "ONE\ntwo\nthree\nfour\nfive\nsix\nSEVEN\n")
    });
    // Undo the subagent's edit only; the main agent's later edit stays.
    p.ok(".", &["undo", "agent:fixer"]);
    assert_eq!(p.read("a.txt"), "one\ntwo\nthree\nfour\nfive\nsix\nSEVEN\n");

    // A later change to the same lines makes the undo refuse.
    p.edit("Edit", "a.txt", "t3", None, || {
        p.write("a.txt", "one\nTWO\nthree\nfour\nfive\nsix\nSEVEN\n")
    });
    let edit = p.log(&[])[0]["seq"].as_i64().unwrap();
    p.write("a.txt", "one\nTwo!\nthree\nfour\nfive\nsix\nSEVEN\n");
    let out = p.run(".", &["undo", &format!("e:{edit}")]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("a.txt"));
    assert_eq!(
        p.read("a.txt"),
        "one\nTwo!\nthree\nfour\nfive\nsix\nSEVEN\n"
    );
}

#[test]
fn commit_stages_only_agent_files_and_guards_mixed_ones() {
    let p = Project::git().init();
    p.write("mine.txt", "user file\n");
    p.prompt("p1");
    p.edit("Write", "agent.txt", "t1", None, || {
        p.write("agent.txt", "agent\n")
    });
    p.edit("Edit", "src/main.rs", "t2", None, || {
        p.write("src/main.rs", "fn main() {}\n")
    });
    p.hook(json!({"hook_event_name": "Stop", "prompt_id": "p1"}));

    let plan = p.json(".", &["commit", "--dry-run"]);
    assert_eq!(plan["paths"], json!(["agent.txt", "src/main.rs"]));
    p.ok(".", &["commit", "-m", "agent work"]);
    let committed = p.git_cmd(&["show", "--name-only", "--format=%s", "HEAD"]);
    assert!(committed.starts_with("agent work"));
    assert!(committed.contains("agent.txt") && committed.contains("src/main.rs"));
    assert!(!committed.contains("mine.txt"));
    assert!(p.git_cmd(&["status", "--short"]).contains("?? mine.txt"));

    // A file the user also touched in the turn needs --include-mixed.
    p.prompt("p2");
    p.edit("Edit", "agent.txt", "t3", None, || {
        p.write("agent.txt", "agent 2\n")
    });
    p.write("agent.txt", "agent 2 + user\n");
    p.ok(".", &["snap"]);
    let out = p.run(".", &["commit", "-m", "x"]);
    assert_eq!(out.status.code(), Some(3));
    p.ok(".", &["commit", "-m", "x", "--include-mixed"]);
    assert_eq!(p.git_cmd(&["status", "--short"]).trim(), "?? mine.txt");
}

#[test]
fn install_writes_hooks_and_skill_idempotently() {
    let p = Project::git();
    p.write(".claude/settings.json", "{\"hooks\": {\"Stop\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"other\"}]}]}, \"model\": \"x\"}");
    p.ok(".", &["install", "claude", "--project"]);
    p.ok(".", &["install", "claude", "--project"]);
    let settings: Value = serde_json::from_str(&p.read(".claude/settings.json")).unwrap();
    assert_eq!(settings["model"], "x");
    let stop = settings["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2, "kept the user's hook, added ours once");
    assert!(
        settings["hooks"]["PostToolUse"][0]["matcher"]
            .as_str()
            .unwrap()
            .contains("Edit")
    );
    assert!(
        p.read(".claude/skills/vibesnap/SKILL.md")
            .starts_with("---\nname: vibesnap")
    );
    p.ok(".", &["install", "claude", "--project", "--uninstall"]);
    let settings: Value = serde_json::from_str(&p.read(".claude/settings.json")).unwrap();
    assert_eq!(settings["hooks"]["Stop"].as_array().unwrap().len(), 1);
    assert!(!p.exists(".claude/skills/vibesnap"));
}
