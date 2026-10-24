#!/usr/bin/env python3
"""Benchmarks vibesnap against the budgets in the roadmap.

Usage: bench.py <vs binary> <work dir> [files]

Builds a tree of N text files (default 20,000, about 6.7 KB each, 134 MiB) plus
50,000 ignored files in node_modules/, commits it to git, and times:
first snapshot (with and without git), no-op snapshot, one-file hook capture,
restore, and store size. Prints a table and writes results.json.
"""
import json, os, random, shutil, statistics, subprocess, sys, time

VS, WORK = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
N = int(sys.argv[3]) if len(sys.argv) > 3 else 20000
ENV = dict(os.environ, HOME=os.path.join(WORK, "home"), XDG_CONFIG_HOME=os.path.join(WORK, "home"),
           GIT_AUTHOR_NAME="b", GIT_AUTHOR_EMAIL="b@b", GIT_COMMITTER_NAME="b", GIT_COMMITTER_EMAIL="b@b")


def run(cmd, cwd, stdin=None):
    t = time.perf_counter()
    r = subprocess.run(cmd, cwd=cwd, env=ENV, input=stdin, capture_output=True, text=True)
    dt = time.perf_counter() - t
    if r.returncode != 0:
        raise SystemExit(f"{cmd} failed: {r.stderr}")
    return dt, r.stdout


def times(cmd, cwd, n, stdin_fn=None, before=None):
    out = []
    for i in range(n):
        if before:
            before(i)
        out.append(run(cmd, cwd, stdin_fn(i) if stdin_fn else None)[0])
    return out


def p(xs, q):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def make_tree(root):
    random.seed(1)
    blob = os.urandom(3400).hex()
    for i in range(N):
        d = os.path.join(root, "d%03d/s%02d" % (i % 200, i % 13))
        os.makedirs(d, exist_ok=True)
        with open(f"{d}/f{i}.txt", "w") as f:
            f.write(blob[i % 3000:i % 3000 + 6700] + str(i) + "\n")
    for i in range(50000):
        d = os.path.join(root, "node_modules/p%03d" % (i % 500))
        os.makedirs(d, exist_ok=True)
        with open(f"{d}/m{i}.js", "w") as f:
            f.write("x")
    with open(os.path.join(root, ".gitignore"), "w") as f:
        f.write("node_modules/\n")


def du(path):
    total = 0
    for dirpath, _, files in os.walk(path):
        for name in files:
            try:
                total += os.lstat(os.path.join(dirpath, name)).st_size
            except FileNotFoundError:
                pass
    return total


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(os.path.join(WORK, "home"))
    results = {}
    tree = os.path.join(WORK, "git")
    make_tree(tree)
    plain = os.path.join(WORK, "plain")
    shutil.copytree(tree, plain)

    results["git add -A"], _ = run(["git", "init", "-q"], tree)
    results["git add -A"], _ = run(["git", "add", "-A"], tree)
    results["git commit"], _ = run(["git", "commit", "-qm", "base"], tree)
    # A real repository's index is older than its files; right after `git add`
    # most entries are racily clean and git itself would re-hash them.
    time.sleep(1.1)
    run(["git", "update-index", "-q", "--refresh"], tree)
    results["git status"] = statistics.median(times(["git", "status", "--short"], tree, 5))

    results["first snapshot (git repo, seeded from index)"], _ = run([VS, "init"], tree)
    results["first snapshot (no git)"], _ = run([VS, "init"], plain)
    noop = times([VS, "snap"], tree, 20)
    results["no-op snapshot (median)"] = statistics.median(noop)
    results["no-op snapshot (p95)"] = p(noop, 0.95)
    results["status (median)"] = statistics.median(times([VS, "status"], tree, 5))

    target = os.path.join(tree, "d000/s00/f0.txt")

    def touch(i):
        with open(target, "a") as f:
            f.write(f"edit {i}\n")

    def payload(event):
        return lambda i: json.dumps({"hook_event_name": event, "session_id": "s", "cwd": tree, "tool_name": "Edit",
                                     "tool_use_id": f"t{i}", "tool_input": {"file_path": target}, "tool_response": {}})

    pre = []
    post = []
    for i in range(100):
        pre.append(run([VS, "hook", "claude"], tree, payload("PreToolUse")(i))[0])
        touch(i)
        post.append(run([VS, "hook", "claude"], tree, payload("PostToolUse")(i))[0])
    results["PreToolUse Edit hook (p95)"] = p(pre, 0.95)
    results["PostToolUse Edit hook (median)"] = statistics.median(post)
    results["PostToolUse Edit hook (p95)"] = p(post, 0.95)
    prompt = times([VS, "hook", "claude"], tree, 10,
                   stdin_fn=lambda i: json.dumps({"hook_event_name": "UserPromptSubmit", "session_id": "s", "cwd": tree, "prompt_id": f"p{i}"}))
    results["UserPromptSubmit hook, no changes (p95)"] = p(prompt, 0.95)
    touch(99)
    results["snapshot after a one-file change"], _ = run([VS, "snap"], tree)

    results["restore 50 events back (1 file differs)"], _ = run([VS, "restore", "@-50"], tree)
    results["restore back"], _ = run([VS, "restore", "@-1"], tree)

    results["store size (.vibe), bytes"] = du(os.path.join(tree, ".vibe"))
    results[".git size, bytes"] = du(os.path.join(tree, ".git"))
    results["store size without git, bytes"] = du(os.path.join(plain, ".vibe"))
    t, _ = run([VS, "gc", "--keep", "7d"], tree)
    results["gc"] = t
    results["store size after gc, bytes"] = du(os.path.join(tree, ".vibe"))
    t, _ = run([VS, "gc", "--keep", "7d"], plain)
    results["store size without git after gc, bytes"] = du(os.path.join(plain, ".vibe"))

    width = max(len(k) for k in results)
    for k, v in results.items():
        shown = f"{v / 1e6:8.1f} MB" if k.endswith("bytes") else f"{v * 1000:8.1f} ms"
        print(f"{k:{width}}  {shown}")
    with open(os.path.join(WORK, "results.json"), "w") as f:
        json.dump(results, f, indent=2)


main()
