# Repository Guidelines

Open Lakehouse Lab — a hands-on environment for experimenting with the Lakehouse
architecture and its open-source stack (Delta, DataFusion, lineage, policy), used
to validate patterns built on `delta-rs`.

## Project structure

Multi-crate Rust workspace (`crates/*`, Edition 2024; MSRV pinned in the
workspace `Cargo.toml` — read it there). Notable crates:

- `client`, `common` — shared client + types
- `datafusion`, `datafusion-cedar` — DataFusion integration; Cedar-policy bridge
- `cedar-oci` — Cedar policy / OCI artifact handling
- `portal`, `hydrofoil` — Connect-RPC services (proto-generated)
- `lineage-service`, `open-lineage` — lineage capture (Marquez/OpenLineage)
- `desktop-host`, `env-modules` — desktop host + environment modules

## Build, test, codegen

Task runner is [`just`](https://just.systems) (`just --list` to discover). Code
is generated from proto via `buf`; generated stubs are committed.

- `just build_policy` — `buf generate` for the policy proto
- `just portal-gen` / `just hydrofoil-gen` — regenerate the portal / hydrofoil
  Connect-RPC stubs (`buf generate` + `cargo fmt -p <crate>`). These need
  one-time local codegen plugins — see the recipe comments in the `justfile`.

Standard `cargo build` / `cargo test` / `cargo clippy` / `cargo fmt` apply.

## Commits, signing, PRs

The commit-message contract and signing flow are machine-wide — see
`~/.claude/CLAUDE.md`. Use the `/commit` skill (`.claude/skills/commit/SKILL.md`):
commit **unsigned** as you go, **sign the branch once before opening a PR**, and
prefer small, well-scoped conventional commits (PR titles are commitlint-checked).

A **pre-commit hook** (`.pre-commit-config.yaml`) runs on every commit: typos,
ruff format/check, cargo-machete (unused deps), rustfmt `--check`, `cargo check
--workspace`, and `buf format`. Expect it to run — fix what it reports (or let it
rewrite, then re-stage) before the commit succeeds.
