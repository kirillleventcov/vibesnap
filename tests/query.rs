//! P3: refs, diff, log, show, blame, overlaps, recap.

mod common;
use common::Project;
use serde_json::{Value, json};

/// A project with one turn: main edits src/main.rs, the "tester" subagent
/// writes tests/t.rs and then rewrites main's line.
fn session() -> Project {
    let p = Project::git().init();
    p.prompt("p1");
    p.edit("Edit", "src/main.rs", "t1", None, || {
        p.write("src/main.rs", "fn main() {\n    println!(\"hello\");\n}\n")
    });
    p.hook(json!({"hook_event_name": "SubagentStart", "prompt_id": "p1", "agent_id": "a1", "agent_type": "tester"}));
    p.edit("Write", "tests/t.rs", "t2", Some(("a1", "tester")), || {
        p.write("tests/t.rs", "#[test]\nfn t() {}\n")
    });
    p.edit("Edit", "src/main.rs", "t3", Some(("a1", "tester")), || {
        p.write("src/main.rs", "fn main() {\n    println!(\"howdy\");\n}\n")
    });
    p.hook(json!({"hook_event_name": "SubagentStop", "prompt_id": "p1", "agent_id": "a1", "agent_type": "tester",
        "last_assistant_message": "wrote a test"}));
    p.hook(json!({"hook_event_name": "Stop", "prompt_id": "p1"}));
    p
}

#[test]
fn diff_defaults_to_this_turn_with_authors() {
    let p = session();
    let d = p.json(".", &["diff", "--stat"]);
    assert_eq!(d["from"], "turn");
    assert_eq!(d["to"], "@");
    let files: Vec<(&str, Vec<&str>)> = d["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap(),
                f["agents"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(
        files,
        vec![
            ("src/main.rs", vec!["main", "tester"]),
            ("tests/t.rs", vec!["tester"])
        ]
    );
    let names = p.ok(".", &["diff", "--name-only"]);
    assert_eq!(names, "M src/main.rs\nA tests/t.rs\n");
}

#[test]
fn diff_prints_hunks_and_respects_the_budget() {
    let p = Project::git().init();
    let long: String = (0..100).map(|i| format!("line {i}\n")).collect();
    p.write("long.txt", &long);
    let text = p.ok(".", &["diff", "--max-lines", "10"]);
    assert_eq!(text.lines().count(), 11);
    assert!(
        text.ends_with("not shown (pass --max-lines, --stat, or narrow with -- <path>)\n"),
        "{text}"
    );
    p.ok(".", &["snap"]);
    p.write("long.txt", &long.replace("line 50\n", "line fifty\n"));
    let text = p.ok(".", &["diff"]);
    assert!(text.contains("@@ -48,7 +48,7 @@"), "{text}");
    assert!(text.contains("-line 50\n+line fifty\n"));
    assert_eq!(
        text.lines().count(),
        3 + 7 + 1,
        "header, hunk header, 3+1+3 context, +1"
    );
}

#[test]
fn diff_between_refs_and_git() {
    let p = session();
    let d = p.json(".", &["diff", "e:1", "agent:tester", "--stat"]);
    assert_eq!(d["files"][0]["path"], "src/main.rs");
    assert_eq!(d["files"].as_array().unwrap().len(), 1);
    let d = p.json(".", &["diff", "git:HEAD", "--name-only"]);
    assert_eq!(d["files"].as_array().unwrap().len(), 2);
    let d = p.json(".", &["diff", "turn..@-1", "--stat"]);
    assert_eq!(d["files"].as_array().unwrap().len(), 2);
    p.write("bin.dat", "\0\x01\x02");
    let d = p.json(".", &["diff"]);
    let bin = d["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "bin.dat")
        .unwrap()
        .clone();
    assert_eq!(bin["binary"], true);
    assert!(
        bin["patch"]
            .as_str()
            .unwrap()
            .starts_with("Binary file bin.dat")
    );
}

#[test]
fn log_filters() {
    let p = session();
    let tester = p.log(&["--agent", "tester"]);
    assert_eq!(tester.len(), 2);
    assert!(tester.iter().all(|e| e["who"] == "tester"));
    assert_eq!(p.log(&["--agent", "main"]).len(), 1);
    assert_eq!(p.log(&["--actor", "user"]).len(), 1);
    assert_eq!(p.log(&["-S", "howdy"]).len(), 1);
    assert_eq!(
        p.log(&["-S", "hello"]).len(),
        2,
        "added by main, removed by tester"
    );
    assert_eq!(p.log(&["--since", "turn"]).len(), 3);
    assert_eq!(p.log(&["agent:tester"]).len(), 2);
    // Paths resolve against the current directory.
    let from_src: Vec<Value> = p
        .ok("src", &["log", "--json", "--", "main.rs"])
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(from_src.len(), 3);
    let t = p.log(&["--", "tests"]);
    assert_eq!(t.len(), 1);
}

#[test]
fn show_files_at_refs_and_events() {
    let p = session();
    assert!(p.ok(".", &["show", "src/main.rs@e:1"]).contains("\"hi\""));
    assert!(p.ok(".", &["show", "src/main.rs@turn"]).contains("\"hi\""));
    assert!(
        p.ok(".", &["show", "src/main.rs@-1"]).contains("\"hello\""),
        "path@-N is N events back"
    );
    assert!(p.ok("src", &["show", "main.rs@@"]).contains("howdy"));
    assert!(
        p.ok(".", &["show", "src/main.rs@agent:tester"])
            .contains("\"hello\"")
    );
    let e = p.json(".", &["show", "e:3"]);
    assert_eq!(e["who"], "tester");
    assert_eq!(e["files"][0]["path"], "tests/t.rs");
    let patch = p.ok(".", &["show", "e:2", "-p"]);
    assert!(
        patch.contains("-    println!(\"hi\");") && patch.contains("+    println!(\"hello\");")
    );
    let out = p.run(".", &["show", "tests/t.rs@e:1"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn blame_attributes_lines_to_events_and_agents() {
    let p = session();
    p.write(
        "src/main.rs",
        "fn main() {\n    println!(\"howdy\");\n}\n// note\n",
    );
    let b = p.json(".", &["blame", "src/main.rs"]);
    let hunks: Vec<(i64, i64, Value, Value)> = b["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            (
                h["start"].as_i64().unwrap(),
                h["end"].as_i64().unwrap(),
                h["who"].clone(),
                h["state"].clone(),
            )
        })
        .collect();
    assert_eq!(hunks[0], (1, 1, json!("user"), json!("recorded")));
    assert_eq!(hunks[1], (2, 2, json!("tester"), json!("recorded")));
    assert_eq!(hunks[2], (3, 3, json!("user"), json!("recorded")));
    assert_eq!(hunks[3], (4, 4, Value::Null, json!("not recorded yet")));
    let one = p.json(".", &["blame", "src/main.rs", "-L", "2,2"]);
    assert_eq!(one["hunks"].as_array().unwrap().len(), 1);
}

#[test]
fn overlaps_report_lines_one_agent_overwrote() {
    let p = session();
    let o = p.json(".", &["overlaps"]);
    let items = o["overlaps"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{o}");
    assert_eq!(items[0]["path"], "src/main.rs");
    assert_eq!(items[0]["written_by"], "main");
    assert_eq!(items[0]["changed_by"], "tester");
    assert_eq!(items[0]["lines"], 1);
}

#[test]
fn recap_summarises_turns_within_budget() {
    let p = session();
    for i in 2..=30 {
        p.prompt(&format!("p{i}"));
        p.edit(
            "Write",
            &format!("f{i}.txt"),
            &format!("w{i}"),
            None,
            || p.write(&format!("f{i}.txt"), "x\n"),
        );
        p.hook(json!({"hook_event_name": "Stop", "prompt_id": format!("p{i}")}));
    }
    let text = p.ok(".", &["recap"]);
    assert!(text.lines().count() <= 40, "{text}");
    assert!(text.contains("turn:30"));
    assert!(text.contains("earlier turns not shown"));
    let last = p.json(".", &["recap", "--turn"]);
    assert_eq!(last["turns"].as_array().unwrap().len(), 1);
    assert_eq!(last["turns"][0]["number"], 30);
}

#[test]
fn time_refs_and_labels() {
    let p = session();
    assert!(p.ok(".", &["show", "src/main.rs@t:-0s"]).contains("howdy"));
    let early = p.run(".", &["show", "src/main.rs@t:2000-01-01T00:00"]);
    assert_eq!(early.status.code(), Some(2), "nothing existed then");
    p.ok(".", &["label", "good", "e:2"]);
    assert!(p.ok(".", &["show", "src/main.rs@good"]).contains("hello"));
    assert_eq!(
        p.run(".", &["label", "good"]).status.code(),
        Some(2),
        "needs --force"
    );
    assert_eq!(
        p.run(".", &["label", "turn"]).status.code(),
        Some(2),
        "reserved name"
    );
    p.ok(".", &["label", "-d", "good"]);
    assert_eq!(p.json(".", &["label"])["labels"], json!([]));
}
