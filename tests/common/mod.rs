#![allow(dead_code)]

use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

pub struct Project {
    dir: tempfile::TempDir,
    home: tempfile::TempDir,
    pub session: String,
}

impl Project {
    /// A git repository with one commit and a `.gitignore`, not yet initialised.
    pub fn git() -> Project {
        let p = Project::bare();
        p.write(".gitignore", "node_modules/\n");
        p.write("src/main.rs", "fn main() {\n    println!(\"hi\");\n}\n");
        p.git_cmd(&["init", "-q"]);
        p.git_cmd(&["add", "-A"]);
        p.git_cmd(&["commit", "-q", "-m", "init"]);
        p
    }

    pub fn bare() -> Project {
        Project {
            dir: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
            session: "s1".into(),
        }
    }

    pub fn init(self) -> Project {
        self.ok(".", &["init"]);
        self
    }

    pub fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root().join(rel)
    }

    pub fn write(&self, rel: &str, content: &str) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap()
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).symlink_metadata().is_ok()
    }

    pub fn git_cmd(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(self.root())
            .env("HOME", self.home.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn command(&self, cwd: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vs"));
        cmd.args(args)
            .current_dir(self.path(cwd))
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path())
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .stdin(Stdio::null());
        cmd
    }

    pub fn run(&self, cwd: &str, args: &[&str]) -> Output {
        self.command(cwd, args).output().unwrap()
    }

    pub fn ok(&self, cwd: &str, args: &[&str]) -> String {
        let out = self.run(cwd, args);
        assert!(
            out.status.success(),
            "{args:?} failed ({:?}): {}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn json(&self, cwd: &str, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.push("--json");
        let stdout = self.ok(cwd, &args);
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("bad JSON from {args:?}: {stdout:?}: {e}"))
    }

    /// `vs log --json` as a list, newest first.
    pub fn log(&self, extra: &[&str]) -> Vec<Value> {
        let mut args = vec!["log", "--json", "-n", "1000"];
        args.extend_from_slice(extra);
        self.ok(".", &args)
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// Sends one hook payload; returns stdout.
    pub fn hook(&self, mut payload: Value) -> String {
        payload["cwd"] = self.root().to_string_lossy().to_string().into();
        if payload.get("session_id").is_none() {
            payload["session_id"] = self.session.clone().into();
        }
        let mut child = self
            .command(".", &["hook", "claude"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0), "hooks always exit 0");
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn prompt(&self, id: &str) {
        self.hook(json!({"hook_event_name": "UserPromptSubmit", "prompt_id": id, "prompt": "x"}));
    }

    /// PreToolUse, then `change`, then PostToolUse, for an edit tool.
    pub fn edit(
        &self,
        tool: &str,
        rel: &str,
        use_id: &str,
        agent: Option<(&str, &str)>,
        change: impl FnOnce(),
    ) {
        let mut base = json!({
            "tool_name": tool, "tool_use_id": use_id,
            "tool_input": {"file_path": self.path(rel).to_string_lossy()},
        });
        if let Some((id, kind)) = agent {
            base["agent_id"] = id.into();
            base["agent_type"] = kind.into();
        }
        let mut pre = base.clone();
        pre["hook_event_name"] = "PreToolUse".into();
        self.hook(pre);
        change();
        let mut post = base;
        post["hook_event_name"] = "PostToolUse".into();
        post["tool_response"] = json!({});
        self.hook(post);
    }

    pub fn ledger_length(&self) -> usize {
        let vibe = self.path(".vibe");
        let out = Command::new("git")
            .arg("--git-dir")
            .arg(vibe)
            .args(["rev-list", "--count", "refs/vibesnap/ledger"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }
}
