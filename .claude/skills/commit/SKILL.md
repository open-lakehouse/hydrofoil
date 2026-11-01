---
name: commit
description: This skill should be used when the user asks to "commit", "prepare a commit", "stage and commit", "commit my changes", or says the work is done and changes should be committed. Lints, formats, stages by name and commits; on GPG-signed setups it commits unsigned and defers signing to one bulk step after the PR is open.
version: 0.3.0
---

# Commit Workflow

Conventions (PR title = release unit, stacks, generated code with its source) live
in [`AGENTS.md`](../../../AGENTS.md). This skill covers the mechanics.

## Workflow

### Step 1 — Auto-fix lint
`cargo clippy --workspace --all-targets --fix --allow-dirty`, then re-run
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and fix
what remains. Lint may rewrite code; always run it before formatting.

### Step 2 — Format and run the pre-commit checks
`cargo fmt --all` (also `cargo fmt -p <crate>` after any `buf generate` step).
Always after lint, since lint rewrites may need reformatting. Then
`pre-commit run --files <changed files>`: typos, ruff, cargo-machete, rustfmt
`--check`, `cargo check --workspace`, `buf format`. With a global
`core.hooksPath` the framework doesn't run as a git hook, so run it by hand. If
it rewrites files, re-run it once; if it still fails, surface the output and stop.

### Step 3 — Stage specific files
Stage only relevant files by name, never `git add -A` / `git add .` (that picks up
`.env`, `.uc-data/`, `spark-warehouse/` and large binaries).

```bash
git add <file1> <file2> ...
```

PRs are squash-merged, so commits inside a PR are for review only. Split them
where it helps a reviewer, but don't fragment. Keep generated stubs in the
**same commit** as the proto change that produced them. If the work should land on
`main` as separate commits, it belongs in separate stack layers (see AGENTS.md),
not separate commits.

### Step 4 — Write the message, then commit
Derive a temp filename from repo + branch to avoid collisions across worktrees:

```bash
REPO=$(basename $(git rev-parse --show-toplevel))
BRANCH=$(git rev-parse --abbrev-ref HEAD | tr '/' '-')
MSG_FILE="/tmp/commit_msg_${REPO}_${BRANCH}.txt"
```

Write the message (`<type>(<scope>): <subject>` / blank line / body: what and why;
the scope is optional here, e.g. `ui`, `deps`, or a crate such as `portal`) to
that path with the **Write tool**, not echo/heredoc: on macOS/zsh a pasted
multiline heredoc leaves the shell in incomplete-input state. Then:

```bash
git commit --no-gpg-sign -F "$MSG_FILE" && rm "$MSG_FILE"
```

`--no-gpg-sign` is only needed when `commit.gpgsign` is on (the GPG PIN needs an
interactive terminal the agent can't drive); it is harmless otherwise.

**Hooks:** the global git hooks (secret scanning) may abort the commit. Surface
the output and stop; don't loop. The `&& rm` only runs on success, so a failed
commit keeps the message file.

### Step 5 — Push and open the PR (don't wait on signing)
Push and open the PR in the same pass. For a stack, `gh stack submit` pushes every
layer and opens/links the PRs.

## Signing — one bulk step at the end (GPG-signed setups only)

Signing rewrites the commits (amend), so the pushed branch needs a
`--force-with-lease` re-push, which is safe on a solo feature branch. Signatures
aren't required to merge, so this can happen any time before merge. Surface ONE
combined command for the user to run (one PIN); don't offer a re-sign after each
commit.

- One commit (HEAD):
  ```bash
  git commit --amend --no-edit -S && git push --force-with-lease
  ```
- Branch:
  ```bash
  git rebase --exec 'git commit --amend --no-edit -S' "$(git merge-base main HEAD)" && git push --force-with-lease
  ```
- Stack: run from the **top** layer. `--update-refs` moves the lower layers' refs
  too (without it only the top branch gets the signed commits). Then re-push all
  layers, listing every layer branch from bottom to top:
  ```bash
  git rebase --update-refs --exec 'git commit --amend --no-edit -S' main \
    && git push --force-with-lease --atomic origin <layer-1> <layer-2> <top-layer>
  ```
- Verify: `git log --format='%h %G? %s' main..HEAD`. Every commit should show `G`.
