# Local wheels for the Marimo image

This directory holds locally-built Python wheels that are not available on any
package index, copied into the image at build time (`COPY wheels/ ./wheels/` into
the builder `WORKDIR /app`, resolved by uv via the `uc-wheels` flat index declared
in `pyproject.toml` — `uv.lock` pins each wheel by a path relative to /app).

## `unitycatalog-client`

The native Unity Catalog client (PyO3/maturin) is built from the sibling
[`unitycatalog-rs`](https://github.com/.../unitycatalog-rs) repo. It is an **abi3**
wheel, so a single wheel per architecture covers every Python >= 3.9 (incl. the
image's 3.13).

To (re)build and drop the wheels here:

```bash
# in the unitycatalog-rs checkout
just build-py-wheels        # builds amd64 + arm64 manylinux_2_28 wheels into dist/

# copy both arch wheels into this dir (run from unitycatalog-rs)
cp dist/unitycatalog_client-*-abi3-manylinux_2_28_*.whl \
   ../open-lakehouse/environments/docker/marimo/wheels/
```

Both the `x86_64` and `aarch64` wheels live here side by side; uv/pip selects the
one matching the build platform by wheel tag. After updating the wheels, refresh
the lock so `uv sync --frozen` stays satisfied:

```bash
# in environments/docker/marimo/
uv lock
```

The `.whl` files themselves are git-ignored (build artifacts); only this README is
tracked. CI to build and publish these wheels is a deferred follow-up.
