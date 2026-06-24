---
name: commit
description: This skill should be used when the user asks to "commit", "prepare a commit", "stage and commit", "commit my changes", or says the work is done and changes should be committed. Commits unsigned (no interactive GPG PIN) and defers signing to a single bulk step before opening a PR.
version: 0.1.0
---

# Commit Workflow

The commit-message contract (types, `AI-assisted-by: Isaac` trailer, granularity)
lives in `~/.claude/CLAUDE.md` — follow it; this skill covers the mechanics.

Commits here are GPG-signed, and the PIN needs an interactive terminal. The agent
**commits unsigned** (`git commit --no-gpg-sign`, which needs no PIN), and the
user **signs once before pushing / opening a PR**.

## Workflow

### Step 1 — Lint & format
```bash
cargo clippy --workspace --all-targets --fix --allow-dirty   # fix what it reports
cargo fmt --all
```
Clippy first (may rewrite code), then fmt. Run `cargo fmt -p <crate>` after any
`buf generate` codegen step.

### Step 2 — Stage specific files (and split commits)
Stage relevant files by name — never `git add -A` / `.`. When the tree spans
multiple logical changes, make **multiple small, well-scoped commits** (one
type/scope each) rather than one mixed commit — signing is one bulk step per
branch, so small commits are free and give release tooling a richer history.
Don't over-fragment; commit generated stubs in the same commit as their source.

```bash
git add <file1> <file2> ...
```

### Step 3 — Write the message, then commit unsigned
```bash
REPO=$(basename $(git rev-parse --show-toplevel))
BRANCH=$(git rev-parse --abbrev-ref HEAD | tr '/' '-')
MSG_FILE="/tmp/commit_msg_${REPO}_${BRANCH}.txt"
```

Write the message there with the **Write tool** (not echo/heredoc). Format per
`~/.claude/CLAUDE.md`. Then commit — no PIN:

```bash
git commit --no-gpg-sign -F "$MSG_FILE" && rm "$MSG_FILE"
```

**Pre-commit hook:** `.pre-commit-config.yaml` runs on every commit (typos, ruff,
cargo-machete, rustfmt `--check`, `cargo check --workspace`, `buf format`) and can
rewrite files or abort. If it rewrites files, re-stage and retry the commit once.
If it still fails, surface the output and stop — don't loop.

### Step 4 — Push and open the PR (don't wait on signing)
Commit unsigned, then **push and open the PR in the same pass** — don't stop to
wait for the GPG PIN mid-flow. Tell the user the commits are unsigned and will be
signed in one bulk step at the end. Don't offer a re-sign after each commit.

## Signing — one bulk step at the end (after the PR is open)

Signing rewrites the commits (amend), so the already-pushed branch needs a
`--force-with-lease` re-push. That's safe on a solo feature branch and preferred
over splitting work across handoffs. Surface ONE combined command for the user to
run (one GPG PIN); signatures aren't required to merge, so this can happen any
time before merge:

- One commit (HEAD):
  ```bash
  git commit --amend --no-edit -S && git push --force-with-lease
  ```
- Range (normal case):
  ```bash
  git rebase --exec 'git commit --amend --no-edit -S' "$(git merge-base main HEAD)" && git push --force-with-lease
  ```
- Verify: `git log --format='%h %G? %s' main..HEAD` — every commit shows `G`.
