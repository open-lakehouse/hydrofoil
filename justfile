# list all commands by default
_default:
    just --list

# run local tauri app
app:
    npm run tauri dev -w app

# run hydrofoil server
hydro:
    RUST_LOG="hydrofoil=debug" cargo run --bin hydrofoil

# run marimo notebook server
notebook:
    cd notebooks && uvx marimo edit --sandbox client.py

services:
    docker compose -p open-lakehouse up -d
