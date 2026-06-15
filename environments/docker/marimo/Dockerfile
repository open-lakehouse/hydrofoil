# Marimo notebook server for the open-lakehouse dev stack.
#
# Multi-stage uv build (marimo's recommended pattern): a builder stage runs
# `uv sync --frozen` into /app/.venv, then a slim runtime stage copies just the
# venv. The repo `notebooks/` dir IS baked into the runtime image so the deployed
# container is self-contained; the dev stack additionally bind-mounts ../notebooks over
# /notebooks (see environments/services/notebooks.yaml) so interactive edits persist to
# the host, matching the host `uvx --directory notebooks/` flow. The build context is the
# REPO ROOT (needed to reach notebooks/), so build-input COPYs use full repo-relative
# paths and Dockerfile.dockerignore trims the context to just what this image needs.
#
# Spark note: pyspark does NOT bundle/vendor a JRE (confirmed against the Spark
# 4.x install docs) — classic PySpark launches a local JVM on the driver, so the
# container itself needs Java. Spark 4.x requires Java 17+. The runtime stage
# installs default-jre-headless, which on debian bookworm is OpenJDK 17 and
# creates the stable /usr/lib/jvm/default-java symlink we point JAVA_HOME at
# (the concrete path is arch-specific: java-17-openjdk-{arm64,amd64}). Without
# it the Spark notebooks fail with "JAVA_HOME is not set".
#
# Jar note: the Spark jars (the UC 0.5 connector built from source, delta-spark,
# openlineage, hadoop-aws) are RESOLVED + BAKED at build time (the uc-connector and
# jars stages below) onto a fixed classpath, replacing the old runtime Ivy resolution.
# The UC 0.5 connector isn't published anywhere, so it's built from branch-0.5 here.
# The published jars resolve from Maven Central by default; pass MAVEN_REPO=<proxy> to
# build behind the corporate firewall (the pins are chosen to be proxy-available).

# ---- UC Spark connector: built from source (branch-0.5) ----------------------
# The deployed demo server is Unity Catalog 0.5, which exposes the catalog-managed
# Delta APIs (/delta/v1 add-commit). The published unitycatalog-spark 0.4.x connector
# does NOT carry that support — it only lives on branch-0.5, unreleased to Maven
# Central, so we build the connector from source here. crossPaths=false +
# CrossSparkVersions => the artifact is io.unitycatalog:unitycatalog-spark_4.1_2.13:0.5.0
# (default sparkVersion 4.1.0). publishLocal drops it into ~/.ivy2/local so the jars
# stage below can pull it as a normal Spark package alongside the Maven-Central ones.
FROM eclipse-temurin:17-jdk AS uc-connector
ARG UC_BRANCH=branch-0.5
ARG UC_SPARK_VERSION=4.1.0
# Optional Maven mirror. The repo's build/sbt wrapper honors MAVEN_PROXY_URL for BOTH the
# sbt-launcher bootstrap download AND as the sole dependency repository (local + proxy) —
# matching the quickstart's documented build flow. Leave empty for an open-internet build
# (the wrapper uses Maven Central); set to the proxy URL to build behind the firewall.
# (The connector's deps — hadoop-aws, scala, etc. — ARE mirrored on the proxy.)
ARG MAVEN_PROXY_URL=
RUN apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN git clone --depth 1 --branch ${UC_BRANCH} \
    https://github.com/unitycatalog/unitycatalog /uc
WORKDIR /uc
# Build + publish the Spark connector AND the UC Java client to the local Ivy repo.
# `spark/publishLocal` alone publishes only the connector; its transitive
# `unitycatalog-client` dep then resolves to the latest RELEASED client (0.4.0) from
# Maven Central — which LACKS classes the branch-0.5 connector calls (e.g.
# io.unitycatalog.client.internal.ApiClientUtils), so managed-table writes fail at
# runtime with ClassNotFoundException. branch-0.5's client is unreleased, so we must
# publishLocal it too (sbt project `client` -> artifact unitycatalog-client) so the jar
# resolver prefers the 0.5 client over Central's 0.4.0.
#
# Skip the client's Javadoc: `publishLocal` runs the `doc` task, and the client's
# javadoc fails — its OpenAPI-generated sources reference packages
# (io.unitycatalog.client.delta.model.*) javadoc can't resolve ("package ... does not
# exist"). We only need the compiled jar + pom for resolution, not the javadoc. Both:
#   - deselect the javadoc artifact from publish (publishArtifact := false), and
#   - make the `doc` task itself a no-op (empty sources) so it can't fail even if some
#     publish path still evaluates it (the client redefines `Compile / doc` in build.sbt).
# Run `client/generate` as its OWN sbt command first. The client's OpenAPI codegen
# writes sources into clients/java/target/src/main/java, which `Compile /
# unmanagedSourceDirectories` references; on a clean checkout the publishLocal compile
# can snapshot that dir BEFORE generate populates it, so only the hand-written sources
# compile and javac fails with `cannot find symbol: ApiClient` (the generated classes).
# Generating in a prior command guarantees the sources exist before compile scans.
# Also publishLocal the `hadoop` module: the connector is `.dependsOn(client, hadoop)`
# and the catalog-managed write path loads io.unitycatalog.hadoop.UCCredentialHadoopConfs
# (UC credential vending) from it — another branch-0.5 module unpublished to Maven, so
# without it managed-table writes fail with ClassNotFoundException at credential vending.
RUN MAVEN_PROXY_URL="${MAVEN_PROXY_URL}" \
    ./build/sbt -DsparkVersion=${UC_SPARK_VERSION} \
      'set client / Compile / packageDoc / publishArtifact := false' \
      'set client / Compile / doc / sources := Seq.empty' \
      "client/generate" "client/publishLocal" "hadoop/publishLocal" "spark/publishLocal"

# ---- Spark jar closure: resolve everything into one classpath dir -------------
# Reuse Spark's own resolver (spark-submit --packages) so the dir holds the exact
# transitive closure --packages would pull. Coordinates pinned to the latest stable
# releases that are reachable through both Maven Central and the Databricks proxy
# (see MAVEN_REPO + the openlineage 1.48.0 note below).
FROM eclipse-temurin:17-jdk AS jars
ARG DELTA_PACKAGE=io.delta:delta-spark_4.1_2.13:4.1.0
ARG UC_PACKAGE=io.unitycatalog:unitycatalog-spark_4.1_2.13:0.5.0
ARG HADOOP_AWS_PACKAGE=org.apache.hadoop:hadoop-aws:3.4.2
ARG OPENLINEAGE_PACKAGE=io.openlineage:openlineage-spark_2.13:1.48.0
ARG PYSPARK_VERSION=4.1.2
# Remote Maven repo for resolution. Defaults to Maven Central. openlineage is pinned to
# 1.48.0 — the highest the Databricks Maven proxy mirrors (1.49.0 is Central-only) — so
# this build also works behind the corporate firewall by overriding MAVEN_REPO to the
# proxy. Bump to 1.49.0+ only once an open-internet/CI build is the norm.
ARG MAVEN_REPO=https://repo1.maven.org/maven2
# pip index for the throwaway pyspark below. Defaults to PyPI; override to the Databricks
# proxy (same value as the builder's UV_INDEX_URL) when building behind the firewall.
ARG PIP_INDEX_URL=https://pypi.org/simple
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 python3-pip python3-venv \
    && rm -rf /var/lib/apt/lists/*
# A throwaway pyspark gives us spark-submit + the matching Spark to resolve against.
RUN python3 -m venv /opt/resolver \
    && /opt/resolver/bin/pip install --no-cache-dir --index-url "${PIP_INDEX_URL}" "pyspark==${PYSPARK_VERSION}"
# Copy the whole Ivy home from the connector build: `local` has the published
# unitycatalog-spark_4.1_2.13:0.5.0, and `cache` already holds the connector's deps that
# sbt resolved through the proxy (so they need not be re-fetched).
COPY --from=uc-connector /root/.ivy2 /root/.ivy2
# Resolve the closure + harvest jars into /spark-jars. Logic lives in a script (kept out of
# the Dockerfile to avoid shell-quoting fragility); it builds a custom ivysettings.xml so
# the resolver chain is exactly `local` + ${MAVEN_REPO} and nothing else. Coordinates +
# MAVEN_REPO are passed through the environment.
#
# NOTE: the build context is the REPO ROOT (so the runtime stage can bake repo-root
# notebooks/), hence all build-input COPYs are prefixed with environments/docker/marimo/.
COPY environments/docker/marimo/resolve-spark-jars.sh /usr/local/bin/resolve-spark-jars.sh
RUN chmod +x /usr/local/bin/resolve-spark-jars.sh \
    && DELTA_PACKAGE="${DELTA_PACKAGE}" UC_PACKAGE="${UC_PACKAGE}" \
       HADOOP_AWS_PACKAGE="${HADOOP_AWS_PACKAGE}" OPENLINEAGE_PACKAGE="${OPENLINEAGE_PACKAGE}" \
       MAVEN_REPO="${MAVEN_REPO}" \
       /usr/local/bin/resolve-spark-jars.sh

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

# Cached dependency layer: lockfile + manifest + the local wheels index. pyproject's
# [[tool.uv.index]] url = "./wheels" resolves unitycatalog-client from this dir, so it
# must be present before `uv sync` (both arch wheels live here; see wheels/README.md).
COPY environments/docker/marimo/pyproject.toml environments/docker/marimo/uv.lock ./
COPY environments/docker/marimo/wheels ./wheels
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-install-project --no-dev

# ---- Runtime: slim image + JRE + the prebuilt venv ---------------------------
FROM python:3.13-slim-bookworm AS runtime

# default-jre-headless for pyspark/delta-spark; tini for clean signal handling.
# nodejs (bookworm ships v18) is required by marimo's language servers: the `ty`
# type-checker LSP (and copilot/basedpyright/pyrefly) are launched via marimo's
# bundled Node multiplexer (_lsp/index.cjs), not as native binaries. Without node
# marimo aborts the ty LSP and shows a misleading "Install ty" error even though
# ty itself is present in the venv.
RUN apt-get update \
    && apt-get install -y --no-install-recommends default-jre-headless tini nodejs \
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

# --- Baked Spark jar classpath -----------------------------------------------
# The Spark jars (UC 0.5 connector, delta-spark, openlineage, hadoop-aws + their
# transitive closure) are resolved at BUILD time and baked in here, instead of the
# old runtime `spark.jars.packages` Ivy resolution through the Databricks Maven proxy
# (which mirrors outdated artifacts — e.g. it 404s openlineage 1.49.0). Notebooks now
# run fully offline with no Ivy resolution.
#
# We point every SparkSession at the baked dir via a spark-defaults.conf written into
# pyspark's bundled conf dir. The OpenLineage SparkListener must be on the DRIVER
# classpath at JVM launch, so we set spark.driver.extraClassPath (a dir glob) in
# addition to spark.jars (executor + general visibility).
COPY --from=jars --chown=appuser:appuser /spark-jars /opt/spark-jars
RUN SPARK_CONF_DIR="$(/app/.venv/bin/python -c 'import pyspark,os;print(os.path.join(os.path.dirname(pyspark.__file__),"conf"))')" \
    && mkdir -p "$SPARK_CONF_DIR" \
    && JARS_CSV="$(ls /opt/spark-jars/*.jar | paste -sd, -)" \
    && printf 'spark.jars %s\nspark.driver.extraClassPath %s\nspark.executor.extraClassPath %s\n' \
        "$JARS_CSV" "/opt/spark-jars/*" "/opt/spark-jars/*" \
        > "$SPARK_CONF_DIR/spark-defaults.conf" \
    && echo "Baked $(ls /opt/spark-jars/*.jar | wc -l) jars into $SPARK_CONF_DIR/spark-defaults.conf"

# Bake the repo notebooks into the image so the DEPLOYED container is self-contained
# (the dev stack still bind-mounts ../notebooks over /notebooks for live edits — the mount
# shadows this baked copy, so local editing is unchanged). Context is the repo root.
WORKDIR /notebooks
COPY --chown=appuser:appuser notebooks /notebooks

USER appuser

EXPOSE 2718

# marimo serves /health (200 once up). No curl in slim -> python3 urllib probe.
HEALTHCHECK --interval=10s --timeout=5s --retries=10 --start-period=20s \
    CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:2718/health')" || exit 1

ENTRYPOINT ["tini", "--"]
# Headless interactive editor, no auth token, listening on all interfaces.
CMD ["marimo", "edit", "--headless", "--host", "0.0.0.0", "-p", "2718", "--no-token"]
