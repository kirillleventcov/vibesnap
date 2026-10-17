//! End-to-end tests that run the binary in a scratch repo. Each one reproduces a
//! finding from the roadmap's gap list (G1...) against the fixed behaviour.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Sandbox {
    dir: tempfile::TempDir,
    home: tempfile::TempDir,
}

impl Sandbox {
    /// A repo whose .gitignore does not mention .vibe, so nothing but the tool
    /// itself keeps the store out of snapshots.
    fn new() -> Sandbox {
        let sandbox = Sandbox::bare();
        sandbox.write(".gitignore", "node_modules/\n");
        sandbox.ok(".", &["init"]);
        sandbox
    }

    fn bare() -> Sandbox {
        Sandbox {
            dir: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap()
    }

    fn command(&self, cwd: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vibesnap"));
        cmd.args(args)
            .current_dir(self.path(cwd))
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path())
            .stdin(Stdio::null());
        cmd
    }

    fn run(&self, cwd: &str, args: &[&str]) -> Output {
        self.command(cwd, args).output().unwrap()
    }

    fn ok(&self, cwd: &str, args: &[&str]) -> String {
        let out = self.run(cwd, args);
        assert!(
            out.status.success(),
            "{args:?} failed: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn json(&self, cwd: &str, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let stdout = self.ok(cwd, &args);
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("bad JSON {stdout:?}: {e}"))
    }

    fn snap(&self, args: &[&str]) -> String {
        let mut full = vec!["snap"];
        full.extend_from_slice(args);
        let out = self.json(".", &full);
        assert_eq!(out["created"], true, "{out}");
        out["id"].as_str().unwrap().to_string()
    }

    fn checkpoints(&self) -> Vec<Value> {
        self.json(".", &["list"])["checkpoints"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn manifest(&self, id: &str) -> Value {
        let text = self.read(&format!(".vibe/snapshots/{id}.json"));
        serde_json::from_str(&text).unwrap()
    }
}

fn manifest_paths(manifest: &Value) -> Vec<String> {
    manifest["files"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[test]
fn g1_store_is_never_snapshotted_and_restore_keeps_history() {
    let s = Sandbox::new();
    s.write("a.txt", "1");
    s.write("b.txt", "1");
    let first = s.snap(&[]);
    s.write("a.txt", "2");
    s.snap(&[]);
    s.write("a.txt", "3");
    let third = s.snap(&[]);

    for id in [&first, &third] {
        let paths = manifest_paths(&s.manifest(id));
        assert_eq!(paths, vec![".gitignore", "a.txt", "b.txt"], "{id}");
    }

    s.json(".", &["restore", &first]);
    assert_eq!(s.read("a.txt"), "1");
    assert_eq!(s.checkpoints().len(), 3);
}

#[test]
fn g1_legacy_snapshot_holding_the_store_cannot_overwrite_it() {
    let s = Sandbox::new();
    s.write("a.txt", "1");
    let id = s.snap(&[]);

    // Older versions could record .vibe/meta.db. Point that entry at a real object.
    let mut manifest = s.manifest(&id);
    let a_hash = manifest["files"]["a.txt"].clone();
    manifest["files"][".vibe/meta.db"] = a_hash.clone();
    manifest["files"]["../outside.txt"] = a_hash;
    fs::write(
        s.path(&format!(".vibe/snapshots/{id}.json")),
        manifest.to_string(),
    )
    .unwrap();

    let out = s.json(".", &["restore", &id]);
    let skipped: Vec<&str> = out["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["path"].as_str().unwrap())
        .collect();
    assert!(skipped.contains(&".vibe/meta.db"), "{out}");
    assert!(skipped.contains(&"../outside.txt"), "{out}");
    assert!(!s.root().parent().unwrap().join("outside.txt").exists());
    // The database survived: the original checkpoint is still listed, next to the
    // state saved before the restore.
    let ids: Vec<Value> = s.checkpoints().iter().map(|c| c["id"].clone()).collect();
    assert!(ids.contains(&Value::from(id.as_str())), "{ids:?}");
}

#[test]
fn g2_restore_saves_edits_and_removes_newer_files() {
    let s = Sandbox::new();
    s.write("a.txt", "v1");
    let base = s.snap(&[]);

    s.write("a.txt", "v2 (never snapped)");
    s.write("src/new.rs", "fn new() {}");

    let out = s.json(".", &["restore", &base]);
    assert_eq!(s.read("a.txt"), "v1");
    assert!(!s.path("src/new.rs").exists());
    assert!(!s.path("src").exists(), "emptied directories are removed");
    assert_eq!(out["deleted"], serde_json::json!(["src/new.rs"]));
    assert_eq!(out["written"], serde_json::json!(["a.txt"]));

    // The edits were saved first, so the restore can be undone.
    let saved = out["saved"]
        .as_str()
        .expect("current state saved")
        .to_string();
    s.json(".", &["restore", &saved]);
    assert_eq!(s.read("a.txt"), "v2 (never snapped)");
    assert_eq!(s.read("src/new.rs"), "fn new() {}");
}

#[test]
fn restore_leaves_unchanged_and_ignored_files_alone() {
    let s = Sandbox::new();
    s.write("a.txt", "v1");
    s.write("same.txt", "same");
    let base = s.snap(&[]);
    s.write("a.txt", "v2");
    s.write("node_modules/dep/index.js", "ignored");

    let before = fs::metadata(s.path("same.txt"))
        .unwrap()
        .modified()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let out = s.json(".", &["restore", &base]);

    let after = fs::metadata(s.path("same.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after, "unchanged file was rewritten");
    assert_eq!(out["unchanged"], 2); // .gitignore and same.txt
    assert_eq!(s.read("node_modules/dep/index.js"), "ignored");
}

#[test]
fn restore_keeps_exec_bit_and_does_not_follow_destination_symlinks() {
    use std::os::unix::fs::PermissionsExt;
    let s = Sandbox::new();
    s.write("run.sh", "echo 1");
    fs::set_permissions(s.path("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    s.write("link.txt", "inside");
    let base = s.snap(&[]);

    s.write("run.sh", "echo 2");
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, "do not touch").unwrap();
    fs::remove_file(s.path("link.txt")).unwrap();
    std::os::unix::fs::symlink(&victim, s.path("link.txt")).unwrap();

    s.json(".", &["restore", &base]);
    assert_eq!(s.read("run.sh"), "echo 1");
    let mode = fs::metadata(s.path("run.sh")).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "exec bit lost");
    assert_eq!(fs::read_to_string(&victim).unwrap(), "do not touch");
    assert_eq!(s.read("link.txt"), "inside");
    assert!(
        !fs::symlink_metadata(s.path("link.txt"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn g3_truncated_object_is_repaired_on_next_snap() {
    use sha2::{Digest, Sha256};
    let s = Sandbox::new();
    let content = "important content that must survive";
    let hash = hex::encode(Sha256::digest(content.as_bytes()));
    fs::create_dir_all(s.path(".vibe/objects")).unwrap();
    fs::write(s.path(&format!(".vibe/objects/{hash}")), &content[..4]).unwrap();

    s.write("a.txt", content);
    let id = s.snap(&[]);
    s.write("a.txt", "changed");
    s.json(".", &["restore", &id]);
    assert_eq!(s.read("a.txt"), content);
}

#[test]
fn g5_parallel_snaps_keep_the_parent_chain() {
    const N: usize = 8;
    let s = Sandbox::new();
    s.write("base.txt", "base");
    s.snap(&[]);
    for i in 0..N {
        s.write(&format!("f{i}.txt"), &i.to_string());
    }

    let children: Vec<_> = (0..N)
        .map(|i| {
            s.command(".", &["snap", &format!("f{i}.txt"), "--json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let checkpoints = s.checkpoints();
    assert_eq!(checkpoints.len(), N + 1);
    assert!(checkpoints[0]["parent"].is_null());
    for pair in checkpoints.windows(2) {
        assert_eq!(pair[1]["parent"], pair[0]["id"], "chain broken");
    }
    let head = checkpoints.last().unwrap();
    assert_eq!(head["current"], true);
    assert_eq!(head["complete"], true);
    assert_eq!(
        manifest_paths(&s.manifest(head["id"].as_str().unwrap())).len(),
        N + 2
    );
}

#[test]
fn g6_partial_snap_keeps_the_rest_of_the_tree() {
    let s = Sandbox::new();
    s.write("a.txt", "1");
    s.write("b.txt", "1");
    s.write("c.txt", "1");
    let full = s.snap(&[]);
    s.write("a.txt", "2");
    s.write("b.txt", "2 (not part of the partial snap)");
    let partial = s.snap(&["a.txt"]);

    let diff = s.json(".", &["diff", &full, &partial]);
    assert_eq!(
        diff["files"],
        serde_json::json!([{"path": "a.txt", "status": "modified", "binary": false, "added": 1, "removed": 1}])
    );

    // Restoring the partial snap keeps c.txt and brings b.txt back to its HEAD state.
    s.json(".", &["restore", &full]);
    let out = s.json(".", &["restore", &partial]);
    assert_eq!(out["deletion_enabled"], true);
    assert_eq!(s.read("a.txt"), "2");
    assert_eq!(s.read("b.txt"), "1");
    assert_eq!(s.read("c.txt"), "1");
}

#[test]
fn partial_snap_without_a_full_base_never_deletes() {
    let s = Sandbox::new();
    s.write("a.txt", "1");
    let partial = s.snap(&["a.txt"]);
    assert_eq!(s.checkpoints()[0]["complete"], false);

    s.write("other.txt", "keep me");
    let out = s.json(".", &["restore", &partial]);
    assert_eq!(out["deletion_enabled"], false);
    assert!(s.path("other.txt").exists());
}

#[test]
fn snap_with_nothing_changed_creates_nothing() {
    let s = Sandbox::new();
    s.write("a.txt", "1");
    let id = s.snap(&[]);
    let out = s.json(".", &["snap"]);
    assert_eq!(out["created"], false);
    assert_eq!(out["id"], id.as_str());
    assert_eq!(s.checkpoints().len(), 1);
}

#[test]
fn snap_of_a_missing_path_is_an_error() {
    let s = Sandbox::new();
    let out = s.run(".", &["snap", "nope.txt"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(s.checkpoints().is_empty());
}

#[test]
fn g8_never_prompts_without_a_terminal() {
    let s = Sandbox::new();
    s.write("a.txt", "1");
    let id = s.snap(&[]);

    for args in [
        vec!["restore"],
        vec!["diff", id.as_str()],
        vec!["switch"],
        vec!["restore", "-i"],
        vec!["reset"],
    ] {
        let out = s.run(".", &args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("not a terminal"), "{args:?}: {stderr}");
    }
    let out = s.run(".", &["restore"]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(&id),
        "lists choices"
    );
    assert!(s.path(".vibe").exists(), "reset ran without --confirm");

    s.ok(".", &["reset", "--confirm"]);
    assert!(!s.path(".vibe").exists());
}

#[test]
fn json_errors_carry_a_kind_and_exit_code() {
    let s = Sandbox::new();
    let out = s.run(".", &["restore", "DEADBEEF", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(err["v"], 1);
    assert_eq!(err["error"]["kind"], "checkpoint_not_found");
}

#[test]
fn g13_paths_resolve_against_the_current_directory() {
    let s = Sandbox::new();
    s.write("src/main.rs", "fn main() {}\n");
    s.write("main.rs", "decoy\n");
    let a = s.snap(&[]);
    s.write("src/main.rs", "fn main() { changed() }\n");

    let out = s.json("src", &["snap", "main.rs"]);
    let b = out["id"].as_str().unwrap().to_string();
    assert_eq!(out["paths"], serde_json::json!(["src/main.rs"]));
    assert_eq!(manifest_paths(&s.manifest(&b)).len(), 3);

    let diff = s.json("src", &["diff", &a, &b, "main.rs"]);
    assert_eq!(diff["files"][0]["path"], "src/main.rs");

    let list = s.json("src", &["list", "--file", "main.rs"]);
    assert_eq!(list["checkpoints"].as_array().unwrap().len(), 2);

    s.json("src", &["restore", &a, "--file", "main.rs"]);
    assert_eq!(s.read("src/main.rs"), "fn main() {}\n");
    assert_eq!(s.read("main.rs"), "decoy\n");

    // Running from a subdirectory with no paths still snaps the whole repo.
    s.write("main.rs", "decoy 2\n");
    let out = s.json("src", &["snap"]);
    assert_eq!(out["paths"], serde_json::json!([]));
    assert_eq!(out["complete"], true);
}

#[test]
fn tracks_switch_round_trip() {
    let s = Sandbox::new();
    s.write("a.txt", "main");
    let main_head = s.snap(&[]);
    s.ok(".", &["branch", "exp"]);
    s.json(".", &["switch", "exp"]);
    s.write("a.txt", "experiment");
    s.write("exp.txt", "only on exp");
    s.snap(&[]);

    s.json(".", &["switch", "main"]);
    assert_eq!(s.read("a.txt"), "main");
    assert!(!s.path("exp.txt").exists());
    let list = s.json(".", &["list"]);
    assert_eq!(list["head"], main_head.as_str());

    s.json(".", &["switch", "exp"]);
    assert_eq!(s.read("exp.txt"), "only on exp");
}

#[test]
fn legacy_repo_is_migrated_and_its_checkpoints_never_delete() {
    use sha2::{Digest, Sha256};
    let s = Sandbox::bare();
    s.write("a.txt", "old");
    s.write("extra.txt", "created later");
    let hash = hex::encode(Sha256::digest(b"old"));
    s.write(&format!(".vibe/objects/{hash}"), "old");
    s.write(
        ".vibe/snapshots/0A0B0C0D.json",
        &format!("{{\n  \"files\": {{\n    \"a.txt\": \"{hash}\"\n  }}\n}}"),
    );
    s.write(".vibe/HEAD", "main 0A0B0C0D\n");
    let conn = rusqlite::Connection::open(s.path(".vibe/meta.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE tracks (name TEXT PRIMARY KEY, head TEXT);
         CREATE TABLE checkpoints (id TEXT PRIMARY KEY, track TEXT, parent TEXT,
             timestamp INTEGER, note TEXT, is_auto INTEGER DEFAULT 0);
         INSERT INTO tracks VALUES ('main', '0A0B0C0D');
         INSERT INTO checkpoints VALUES ('0A0B0C0D', 'main', NULL, 1700000000, 'old', 0);",
    )
    .unwrap();
    drop(conn);

    let list = s.json(".", &["list"]);
    assert_eq!(list["head"], "0A0B0C0D");
    assert!(list["checkpoints"][0]["complete"].is_null());
    assert!(!s.path(".vibe/HEAD").exists());

    s.write("a.txt", "new");
    let out = s.json(".", &["restore", "0a0b0c0d"]);
    assert_eq!(out["deletion_enabled"], false);
    assert_eq!(s.read("a.txt"), "old");
    assert_eq!(s.read("extra.txt"), "created later");
}

#[test]
fn init_twice_fails_once() {
    let s = Sandbox::new();
    let out = s.run(".", &["init"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(stderr.matches("already exists").count(), 1, "{stderr}");
}
