# MLflow + OpenTelemetry auto-instrumentation for the desktop stack.
#
# The stock `mlflow:*-full` image doesn't ship the OTel zero-code tooling, so its
# server process emits no spans of its own. This thin shim adds the distro + HTTP
# OTLP exporter and makes `opentelemetry-instrument` the entrypoint, so the
# existing `command:` (mlflow server ...) runs auto-instrumented. Behaviour is
# driven entirely by OTEL_* env (set in services/desktop/mlflow.yaml); with no
# OTEL_EXPORTER_OTLP_ENDPOINT the instrumentation is a no-op, so the image is safe
# to run without a collector.
#
# Build: docker build -t open-lakehouse/mlflow-otel:local environments/docker/mlflow-otel
#
# Package index follows the repo convention (see docker/marimo/Dockerfile):
# defaults to the internal Databricks PyPI proxy; override for off-network builds
# with --build-arg PIP_INDEX_URL=https://pypi.org/simple.
FROM ghcr.io/mlflow/mlflow:v3.12.0-full

ARG PIP_INDEX_URL=https://pypi-proxy.cloud.databricks.com/simple
RUN pip install --no-cache-dir --index-url "${PIP_INDEX_URL}" \
        opentelemetry-distro \
        opentelemetry-exporter-otlp-proto-http \
 && opentelemetry-bootstrap -a install

# `opentelemetry-instrument` wraps whatever command/args follow; the compose
# fragment supplies `mlflow server ...` as before, now auto-instrumented.
ENTRYPOINT ["opentelemetry-instrument"]
CMD ["mlflow", "server"]
