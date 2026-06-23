# Local wheels for the desktop marimo notebook environment

This directory holds locally-built Python wheels that are not on any package
index. The desktop notebook host injects the wheel into marimo's `uvx`
environment by **direct path** (`--with "unitycatalog_client[obstore] @
/abs/path.whl"`) so notebook cells can import it — see `uc_client_wheel` in
`node/desktop/src-tauri/src/notebook.rs`. (A direct path is used, not a
find-links index, because PyPI hosts an unrelated package of the same name that
would otherwise win resolution.) Override the location with the
`OPEN_LAKEHOUSE_UC_WHEELS` environment variable.

## `unitycatalog-client`

The native Unity Catalog client (PyO3/maturin) is built from the sibling
`unitycatalog-rs` repo. With the `[obstore]` extra it powers the DuckDB/Polars
notebook templates' UC-vended `obstore` store (`store_for_volume`), which marimo's
Files panel auto-discovers as a browsable remote source.

Unlike the Docker image (which needs cross-compiled `manylinux` wheels — see
`environments/docker/marimo/wheels/`), the desktop runs marimo natively, so this
wheel must be built for **this host** (no `--zig`). The wheel is **abi3** (pyo3
`abi3-py39`), so one wheel covers every Python >= 3.9.

To (re)build it into this directory:

```bash
# from node/
just build-uc-wheel
```

Run once for a dev checkout, and again after updating the sibling client. Until a
wheel is present, the obstore-backed engines (DuckDB/Polars) fail to import in a
notebook, but the editor and the Spark engine still work.

The `.whl` files are git-ignored (build artifacts); only this README is tracked.
Bundling a prebuilt wheel into the packaged app is a deferred follow-up.
