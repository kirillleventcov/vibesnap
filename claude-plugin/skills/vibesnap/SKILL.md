---
name: vibesnap
description: Look back through this project's file-change history, which vibesnap records automatically. Use it to see what a file looked like earlier, what changed this turn or session, or which agent or turn changed some lines. Use it to undo specific edits without touching others, to recover what you changed after context compaction, and to commit a turn's work when the user asks.
allowed-tools: Bash(vs status:*), Bash(vs diff:*), Bash(vs log:*), Bash(vs show:*), Bash(vs blame:*), Bash(vs recap:*), Bash(vs overlaps:*)
---

# vibesnap: the project's edit history

Every file change here is already recorded: yours, your subagents', shell commands', and the user's own edits.
You never need to init, snapshot or label anything. When unsure what changed, ask the history.
That is cheaper and more reliable than re-reading files or trusting memory.

## When to look
- Before you summarise your work: `vs diff turn --stat`
- A file is not what you remember: `vs log -- <path>`, then `vs show <path>@e:<N>`
- Something broke and you do not know which edit did it: `vs diff turn-1 --stat`, then narrow by path
- After compaction or a resumed session: `vs recap`
- After parallel subagents: `vs log --since turn --stat` and `vs overlaps --since turn`
- The user says "undo that", "go back" or "how was it before": find the ref, then use `vs undo` or `vs restore`

## Look cheaply
1. Look wide first, with `--stat` or `--name-only`.
2. Then look deep: `vs diff <ref> -- <one path>` or `vs show <path>@<ref>`.
3. Never dump a whole-tree patch. Output stops at `--max-lines` (default 400) and says how to see more.
4. Add `--json` when you want to parse the result.

## Refs
- `@` the working tree now; `@-N` N events back; `e:N` right after event N
- `turn` start of this turn; `turn-1` start of the previous turn; `turn:14` start of turn number 14
- `session` start of this session; `agent:<type>` start of that subagent's last run
- `t:14:05`, `t:-10m` the state at a time; `<label>`; `git:HEAD` a git commit
- `A..B` a range; a file at a state is `path@ref`, e.g. `src/auth.ts@turn`

## Undo
- Prefer `vs undo <e:N|@-1|turn|agent:X> [-- paths]` to re-editing by hand. It keeps later edits made by others.
- Exit code 3 means a conflict. Read the overlap it prints and decide; never force.
- `vs restore <ref> [-- paths]` moves files to a state. Use it only when the user asks to go back to a point in time.
- Both record the current state first, so both can themselves be undone.

## Commit
- Only when the user asks: run `vs commit turn -m "<message>"`. Write the message yourself from the conversation and `vs diff turn --stat`.
- It commits only the files agents changed this turn. If it reports files the user also edited, ask before passing `--include-mixed`.

## Do not
- Run `vs init`, `vs snap`, `vs label`, `vs gc` or `vs reset` unless the user asks.
- Commit on your own initiative. The Stop hook already offers it to the user.
