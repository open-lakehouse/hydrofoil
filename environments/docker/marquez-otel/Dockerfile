# Marquez + OpenTelemetry Java agent for the desktop stack.
#
# Bakes the OTel Java agent into the Marquez image so the JVM can attach it via
# JAVA_TOOL_OPTIONS (set in services/desktop/marquez.yaml). Baking (vs. a runtime
# volume mount) keeps the fragment self-contained and the agent version pinned.
# Tracing behaviour is driven entirely by OTEL_* env; with no
# OTEL_EXPORTER_OTLP_ENDPOINT the agent is a no-op, so the image runs fine without
# a collector.
#
# Build: docker build -t open-lakehouse/marquez-otel:local environments/docker/marquez-otel
FROM marquezproject/marquez:0.50.0

# Pin a known-good agent version (2.x) rather than `latest` for reproducibility.
ARG OTEL_AGENT_VERSION=2.11.0
ADD https://github.com/open-telemetry/opentelemetry-java-instrumentation/releases/download/v${OTEL_AGENT_VERSION}/opentelemetry-javaagent.jar /otel/opentelemetry-javaagent.jar
