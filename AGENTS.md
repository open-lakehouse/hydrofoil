# Agent Instructions

Instructions for AI coding agents working in this repository. This is the single
canonical file: `CLAUDE.md` imports it, and Cursor/Codex read it directly. The
walkthrough lives in [`WALKTHROUGH.md`](WALKTHROUGH.md).

Open Lakehouse Lab: a hands-on environment for experimenting with the Lakehouse
architecture and its open-source stack (Delta, DataFusion, lineage, policy), used
to validate patterns built on `delta-rs`.

## Project map

Multi-crate Rust workspace (`crates/*`; edition and MSRV in the workspace
`Cargo.toml`):

- `client` (`hydrofoil-client`), `common` (`hydrofoil-common`): shared client
  and types
- `datafusion` (`deltalake-datafusion`): DataFusion integration
- `portal`, `hydrofoil`: Connect-RPC services (proto-generated)
- `desktop-host`: in-process service executors for the desktop shell
- `node/`: UI, uc-client and desktop workspace (`just node --list`)

Code from sibling repos:

- Cedar/policy crates (`olai-cedar-oci`, `olai-datafusion-policy*`): the
  [breakwater](https://github.com/open-lakehouse/breakwater) repo
- Lineage: the OpenLineage ingest/read service lives in the sibling `headwaters`
  repo; the DataFusion integration is `datafusion-openlineage`
  (datafusion-contrib)
- Unity Catalog crates (`olai-uc-*`) come from
  [mangrove](https://github.com/open-lakehouse/mangrove), and `deltalake-core`
  from a delta-rs fork
- `olai-http` from trestle (crates.io)

## Commands

Task runner is [`just`](https://just.systems) (`just --list`). Code is generated
from proto via `buf`; generated stubs are committed.

| Command | Purpose |
|---|---|
| `just hydro` | Run the hydrofoil server (`HYDROFOIL_CONFIG=…`) |
| `just portal-gen` / `just hydrofoil-gen` | Regenerate the portal / hydrofoil Connect-RPC stubs (`buf generate` + `cargo fmt -p <crate>`). One-time plugin install: see the recipe comments |
| `just build-docker <bin>` | Build a service image |
| `just crates-proxy-patch <repo>` / `just crates-proxy-lock` | Build against a fresh sibling release (see below) |

## Hard rules

- **Never hand-edit generated stubs.** Change the proto, regenerate, and commit
  the output in the same commit as its source change.
- **Keep the mangrove and delta-rs revs in lock-step** with mangrove's workspace
  `Cargo.toml` (see the comment above those dependencies).
- **Never commit to `main`.** Stage files by name; never `git add -A` /
  `git add .`.

## Testing

Changes ship with tests. Never delete or comment out a passing test to make a
suite green. CI runs nextest (`.config/nextest.toml`, profile `ci`) plus
doctests.

### Pre-push check (mirrors CI)

```bash
cargo fmt --all --check \
  && cargo clippy --workspace --all-targets --all-features -- -D warnings \
  && cargo nextest run --workspace --all-features \
  && cargo test --workspace --all-features --doc
```

`.pre-commit-config.yaml` (typos, ruff, cargo-machete, rustfmt `--check`,
`cargo check --workspace`, `buf format`) does **not** run as a git hook while a
global `core.hooksPath` is set. Run `pre-commit run --files <changed files>` by
hand before committing.

## Branches and PRs

- **PRs are squash-merged. The PR title is the commit that lands on `main`**: a
  conventional commit (`feat` `fix` `refactor` `docs` `test` `chore` `perf`
  `build` `ci`, `!` for breaking), linted by `pr-title.yml`. Commits inside a PR
  are for review only.
- **One coherent change = one PR.** Work that should land as separate commits
  goes in a [`gh stack`](https://github.com/github/gh-stack), one layer per
  `type(scope)`.
- **Re-trigger CI with an empty commit** (`git commit --allow-empty`), never by
  closing and reopening the PR.
- Commits are GPG-signed. Agents commit unsigned and the branch is signed once at
  the end; see the machine-wide signing flow in `~/.claude/CLAUDE.md`.

## Builds and disk

Dev builds keep line tables for workspace crates and no debug info for
dependencies (`[profile.dev]` in `Cargo.toml`). Panic locations and backtrace
file:line for workspace code are kept. For a debugger, use
`cargo build --profile debugging` (full debug info, separate `target/debugging/`).
Run `cargo clean` in a worktree before removing it.

## Sibling crates and the crates.io proxy

Local Cargo resolves crates.io through a proxy that hides releases for 7 days.
trestle's [`docs/crates-proxy.md`](https://github.com/open-lakehouse/trestle/blob/main/docs/crates-proxy.md)
has the full guide. In this repo:

- **Git pins** (strategy 3): breakwater and `datafusion-openlineage` are pinned
  to a git rev while their release isn't on the proxy yet. Each keeps its
  `version`, so once that release is older than 7 days, drop `git`/`rev`
  (`grep -n 'git = ' Cargo.toml`). The mangrove `olai-uc-*` crates and
  `deltalake-core` stay on git by design (see the `Cargo.toml` comment).
- **crates.io crates** (`olai-http`): to use a release under 7 days old, run
  `just crates-proxy-patch trestle`, build, then `just crates-proxy-lock` before
  committing `Cargo.lock`. `scripts/crates-proxy.sh` runs trestle's
  `scripts/crates-proxy/` from a trestle checkout next to this repo (or
  `TRESTLE_ROOT`). Never commit `.cargo/` or a lock that
  `scripts/crates-proxy.sh check-lock` rejects.
