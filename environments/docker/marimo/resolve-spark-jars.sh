#!/usr/bin/env bash
# Resolve the Spark jar closure into /spark-jars at image-build time (the `jars` stage of
# the marimo Dockerfile). Reuses Spark's own Ivy resolver so the dir holds exactly what
# `--packages` would pull, then harvests the jars from the Ivy CACHE (Spark 4.x doesn't
# reliably populate the flattened ~/.ivy2/jars retrieve dir).
#
# Resolver chain is a custom ivysettings.xml with ONLY: the `local` resolver (for the
# source-built UC connector in ~/.ivy2/local) + one Maven repo (${MAVEN_REPO}). This
# excludes spark-submit's default repo1.maven.org / repos.spark-packages.org, which would
# otherwise be tried (and fail behind a firewall) even when --repositories is set.
#
# Env (all required, set as ARG->ENV in the Dockerfile):
#   DELTA_PACKAGE UC_PACKAGE HADOOP_AWS_PACKAGE OPENLINEAGE_PACKAGE  -- Maven coordinates
#   MAVEN_REPO                                                       -- remote repo root
set -euo pipefail

# Pin an explicit Ivy home so we know exactly where the resolved jars land. The custom
# ivysettings declares its own <caches> at $IVY_HOME/cache and the resolver chain =
# `local` (the source-built connector) + ${MAVEN_REPO}, and NOTHING else (so spark-submit
# never tries the firewalled repo1.maven.org / repos.spark-packages.org defaults).
IVY_HOME=/tmp/ivyhome
mkdir -p "$IVY_HOME/cache"
# The source-built connector was published to /root/.ivy2/local — point the local resolver there.
cat > /tmp/ivysettings.xml <<XML
<ivysettings>
  <settings defaultResolver="chain"/>
  <caches defaultCacheDir="${IVY_HOME}/cache"/>
  <resolvers>
    <chain name="chain" returnFirst="true">
      <ibiblio name="local-ivy" root="file:///root/.ivy2/local" m2compatible="false"
               pattern="[organisation]/[module]/[revision]/[type]s/[artifact](-[classifier]).[ext]"/>
      <ibiblio name="maven-remote" root="${MAVEN_REPO}" m2compatible="true"/>
    </chain>
  </resolvers>
</ivysettings>
XML

echo "import sys; sys.exit(0)" > /tmp/noop.py
# pyspark lives in the throwaway resolver venv created in the Dockerfile.
SPARK_HOME=$(/opt/resolver/bin/python -c "import pyspark,os;print(os.path.dirname(pyspark.__file__))")

"$SPARK_HOME/bin/spark-submit" \
  --packages "${DELTA_PACKAGE},${UC_PACKAGE},${HADOOP_AWS_PACKAGE},${OPENLINEAGE_PACKAGE}" \
  --conf "spark.jars.ivy=${IVY_HOME}" \
  --conf "spark.jars.ivySettings=/tmp/ivysettings.xml" \
  /tmp/noop.py

# Harvest from the RETRIEVE dir only: Spark flattens the complete resolved closure (incl.
# the source-built connector) into $IVY_HOME/jars with org-prefixed names. Using just this
# dir avoids the duplicate copies you'd get by also scanning $IVY_HOME/cache (same jars,
# unprefixed names) — duplicates on the classpath are wasteful and load each class twice.
mkdir -p /spark-jars
# No -n needed: a single flat dir has no same-named collisions, and dropping it avoids
# coreutils' "behavior of -n is non-portable" warning.
find "$IVY_HOME/jars" -type f -name '*.jar' \
  ! -name '*-sources.jar' ! -name '*-javadoc.jar' \
  -exec cp {} /spark-jars/ \;

count=$(find /spark-jars -name '*.jar' | wc -l)
echo "Collected ${count} jars into /spark-jars"

# Fail loudly if any of the headline artifacts is missing. The UC client is included
# because the branch-0.5 connector calls classes (io.unitycatalog.client.internal.*)
# that only exist in the source-built 0.5 client — a 0.4.0 client from Maven Central
# resolves fine but breaks at runtime.
for m in unitycatalog-spark unitycatalog-client delta-spark openlineage-spark hadoop-aws; do
  ls /spark-jars | grep -q "$m" || { echo "MISSING jar: $m" >&2; exit 1; }
done

# Guard against the version-mismatch regression: the UC client jar MUST carry
# ApiClientUtils (present in 0.5, absent in 0.4.0). Without this, managed-table writes
# fail at runtime with ClassNotFoundException despite resolution "succeeding".
client_jar=$(find /spark-jars -name 'io.unitycatalog_unitycatalog-client-*.jar' | head -1)
[ -n "$client_jar" ] || { echo "MISSING jar: unitycatalog-client" >&2; exit 1; }
# `jar` ships with the JDK base image (no unzip needed).
if ! jar tf "$client_jar" 2>/dev/null | grep -q 'io/unitycatalog/client/internal/ApiClientUtils'; then
  echo "WRONG unitycatalog-client: $(basename "$client_jar") lacks ApiClientUtils — the 0.4.0 client leaked in instead of the source-built 0.5 client (did client/publishLocal run?)." >&2
  exit 1
fi
echo "UC client OK: $(basename "$client_jar") carries ApiClientUtils"
