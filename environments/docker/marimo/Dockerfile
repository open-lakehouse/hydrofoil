# Marimo notebook server for the open-lakehouse dev stack.
#
# Multi-stage uv build (marimo's recommended pattern): a builder stage runs
# `uv sync --frozen` into /app/.venv, then a slim runtime stage copies just the
# venv. Notebooks are NOT baked in — the repo `notebooks/` dir is bind-mounted at
# runtime (see environments/services/notebooks.yaml) so interactive edits persist
# to the host, matching the host `uvx --directory notebooks/` flow.
#
# Spark note: pyspark does NOT bundle/vendor a JRE (confirmed against the Spark
# 4.x install docs) — classic PySpark launches a local JVM on the driver, so the
# container itself needs Java. Spark 4.x requires Java 17+. The runtime stage
# installs default-jre-headless, which on debian bookworm is OpenJDK 17 and
# creates the stable /usr/lib/jvm/default-java symlink we point JAVA_HOME at
# (the concrete path is arch-specific: java-17-openjdk-{arm64,amd64}). Without
# it the Spark notebooks fail with "JAVA_HOME is not set".

# ---- Builder: resolve + install deps into a venv -----------------------------
FROM python:3.13-slim-bookworm AS builder

# Pinned uv (matches marimo's "COPY --from=ghcr.io/astral-sh/uv:<pinned>" guidance).
COPY --from=ghcr.io/astral-sh/uv:0.9.7 /uv /bin/uv

# Package index. The host's uv config points at an internal Databricks pypi proxy
# (~/.config/uv/uv.toml) but that config isn't present inside the build, so set it
# explicitly. Override at build time with --build-arg UV_INDEX_URL=... (e.g. plain
# https://pypi.org/simple when building outside the Databricks network).
ARG UV_INDEX_URL=https://pypi-proxy.cloud.databricks.com/simple
ENV UV_INDEX_URL=${UV_INDEX_URL} \
    UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy \
    UV_PYTHON_DOWNLOADS=never

WORKDIR /app

# Cached dependency layer: lockfile + manifest only, no project source.
COPY pyproject.toml uv.lock ./
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-install-project --no-dev

# ---- Runtime: slim image + JRE + the prebuilt venv ---------------------------
FROM python:3.13-slim-bookworm AS runtime

# default-jre-headless for pyspark/delta-spark; tini for clean signal handling.
RUN apt-get update \
    && apt-get install -y --no-install-recommends default-jre-headless tini \
    && rm -rf /var/lib/apt/lists/*

# Non-root user (UID 1000). On macOS/Docker Desktop the file-share layer maps
# host ownership transparently so this user can write the bind-mounted notebooks.
# On native Linux, override with `user: "${UID}:${GID}"` in the service file.
RUN useradd --create-home --uid 1000 appuser

# Venv on PATH; JAVA_HOME for Spark (Debian default-jre-headless install path).
ENV PATH="/app/.venv/bin:${PATH}" \
    JAVA_HOME="/usr/lib/jvm/default-java" \
    VIRTUAL_ENV="/app/.venv" \
    PYTHONUNBUFFERED=1 \
    MARIMO_SKIP_UPDATE_CHECK=1

COPY --from=builder --chown=appuser:appuser /app/.venv /app/.venv

# Working dir == the runtime bind-mount target for the host notebooks/ dir.
WORKDIR /notebooks
RUN chown appuser:appuser /notebooks

USER appuser

EXPOSE 2718

# marimo serves /health (200 once up). No curl in slim -> python3 urllib probe.
HEALTHCHECK --interval=10s --timeout=5s --retries=10 --start-period=20s \
    CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:2718/health')" || exit 1

ENTRYPOINT ["tini", "--"]
# Headless interactive editor, no auth token, listening on all interfaces.
CMD ["marimo", "edit", "--headless", "--host", "0.0.0.0", "-p", "2718", "--no-token"]
