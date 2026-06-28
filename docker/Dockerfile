# syntax=docker/dockerfile:1
#
# Parameterized build for the workspace's service binaries. Select the binary
# with `--build-arg BIN=hydrofoil` (currently the only service binary).
#
# All workspace deps (including the delta-rs / unitycatalog-rs forks) resolve
# from git or crates.io, so the build context is just this repo — no sibling
# checkouts are needed. cargo-chef caches the dependency graph as a separate
# layer so source edits don't trigger a full dependency rebuild.
ARG RUST_TAG=1.96-bookworm

FROM rust:${RUST_TAG} AS chef
# protoc is needed at build time by transitive build scripts (e.g. cedar-policy's
# prost-build for hydrofoil).
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
ARG NO_CHEF=false
ENV NO_CHEF=${NO_CHEF}
# Optional sparse-registry mirror (e.g. an internal crates proxy). Configured in
# the shared base so every downstream stage — cargo-chef install, prepare, cook,
# and the final build — routes crates-io fetches through it. `git` deps still go
# straight to their host, so the build host needs outbound git access regardless.
ARG CRATES_PROXY=
RUN if [ -n "${CRATES_PROXY}" ]; then \
      proxy_url="${CRATES_PROXY}"; \
      case "${proxy_url}" in \
        sparse+*) ;; \
        *) proxy_url="sparse+${proxy_url}" ;; \
      esac; \
      mkdir -p /usr/local/cargo; \
      printf '[source.crates-io]\nreplace-with = "proxy"\n\n[source.proxy]\nregistry = "%s"\n' "${proxy_url}" > /usr/local/cargo/config.toml; \
      echo "Using CRATES_PROXY for cargo registry: ${proxy_url}"; \
    fi
RUN $NO_CHEF || cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY . .
RUN ($NO_CHEF && touch recipe.json) || cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG BIN
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies only — this is the cached layer.
RUN $NO_CHEF || cargo chef cook --release --recipe-path recipe.json --bin ${BIN}
# Build the application.
COPY . .
RUN cargo build --release --bin ${BIN}

# Minimal runtime: distroless cc (glibc + openssl) for the dynamically-linked
# binary, nonroot. No shell/package manager — run healthchecks from compose.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
ARG BIN
ARG EXPIRES=Never
LABEL org.opencontainers.image.title="${BIN}" quay.expires-after="${EXPIRES}"
# Copy to a fixed name so ENTRYPOINT (exec form) needn't interpolate a build ARG.
COPY --from=builder /app/target/release/${BIN} /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]
