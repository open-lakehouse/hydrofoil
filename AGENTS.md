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
- `datafusion-unitycatalog` (`olai-uc-datafusion`, unpublished): the DataFusion
  catalog provider + catalog-managed Delta integration backed by Unity Catalog
  (moved from mangrove)
- `portal`, `hydrofoil`: Connect-RPC services (proto-generated)
- `wasm/`: a **separate** Cargo workspace (own `Cargo.lock`, arrow-rs /
  object_store / delta-kernel fork pins) for the in-browser query engine
  (`query-wasm`, `olai-delta-df`; moved from mangrove). Build and lint it from
  inside `wasm/` (`just setup-wasm`, `just build-query-wasm`); CI gates it in the
  `query-wasm` job.
- `node/`: UI and uc-client workspace (`just node --list`). The desktop app
  moved to [luminescent-lakehouse](https://github.com/open-lakehouse/luminescent-lakehouse)
  (`apps/desktop`). `node/query-wasm` and `node/files-wasm` package the wasm
  engine as a `QueryRunner` / `FilesRunner` (not in the npm workspace yet; see
  their READMEs).

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
- **Keep the `olai-uc-*` versions on one mangrove release** (mixing releases
  splits the UC types), and the `buoyant_kernel` version on the one `deltalake-core`
  resolves.
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
That applies to every crate a sibling repo publishes, whoever publishes it:
trestle, mangrove, breakwater, datafusion-openlineage. trestle's
[`docs/crates-proxy.md`](https://github.com/open-lakehouse/trestle/blob/main/docs/crates-proxy.md)
has the full guide (trestle only hosts the tooling). In this repo:

- **Git pins** (strategy 3): breakwater and `datafusion-openlineage` are pinned
  to a git rev while their release isn't on the proxy yet. Each keeps its
  `version`, so once that release is older than 7 days, drop `git`/`rev`
  (`grep -n 'git = ' Cargo.toml`). To switch sooner, treat it as a fresh release
  (next item). `deltalake-core` stays on git until delta-rs 2.0 ships the
  `to_delta_predicate` export (see the `Cargo.toml` comment).
- **crates.io crates** (`olai-http`, mangrove's `olai-uc-*`, and any unpinned
  sibling crate):
  to use a release under 7 days old, run `just crates-proxy-patch <upstream>`
  (`trestle`, `breakwater`, `datafusion-openlineage`, …), build, then
  `just crates-proxy-lock` before committing `Cargo.lock`. If the release chains
  on another fresh one, patch both upstreams. `scripts/crates-proxy.sh` runs
  trestle's `scripts/crates-proxy/` and finds the upstream checkouts next to this
  repo (or via `<REPO>_ROOT`). Never commit `.cargo/` or a lock that
  `scripts/crates-proxy.sh check-lock` rejects.
