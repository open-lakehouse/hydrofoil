# 0017 — Environment capabilities, providers, and effects

> Status: **Accepted** (2026-06). Model in `crates/env-modules`, wired into the desktop
> lifecycle (`node/desktop/src-tauri/src/modules.rs`, `lib.rs`) and the in-process engine
> (`crates/desktop-host`). Builds on [`0015`](0015-client-environment-scope.md) (client-side
> environment scope) and [`0016`](0016-local-environment-key-management.md) (per-env KEK);
> relates to [`0011`](0011-uc-credential-vending-server-token.md) (UC credential vending) and
> [`0012`](0012-client-forwarded-lineage-metadata.md) (lineage metadata).

## Context

A desktop environment can run optional services alongside its Unity Catalog sidecar. The
first cut modelled these as **modules** = runnable services (a Docker Compose fragment + a
`requires` dependency edge), resolved into an ordered graph and brought up/down with the
environment lifecycle.

That model is too low-level for what users actually want to express, and it misses the
cross-service wiring that makes a service *useful*:

1. **Users think in capabilities, not technologies.** "I want lineage / observability /
   model tracking / object storage" — not "I want a Marquez container." A capability may be
   satisfied by different technologies (object storage = Azurite *or* SeaweedFS/S3), and the
   same technology can serve multiple capabilities (MLflow is both model tracking *and* an
   OTLP observability sink).

2. **A capability is more than starting containers.** Turning on lineage means the lineage
   *sink* runs **and** our in-process Hydrofoil engine is configured to emit OpenLineage to
   it. Observability means Hydrofoil's OTLP traces point at the sink. Object storage means a
   bucket is created for MLflow — and, optionally, the bucket's credentials are vended into
   Unity Catalog as an External Location + Storage Credential. These "follow-up actions" are
   all the same *shape*: pass a piece of information around, or create a management object.

3. **The wiring is a graph with payloads.** "Lineage produces an endpoint; Hydrofoil consumes
   it" is a dependency edge that carries data. This is exactly the connectivity the future
   environment-graph visualization should render.

Moving External Locations + Credentials into the environment manager ([`0015`]) was an early
instance of this: those are *effects of a capability*, not services.

## Decision

Introduce three layers in `crates/env-modules`, separating user intent from technology from
wiring:

```
Capability   (UI intent)        Lineage | Observability | ModelTracking | ObjectStorage
   │  satisfied by one Provider (default now; user-swappable later)
   ▼
Provider     (technology)       Marquez | MlflowOtlp | Azurite | (SeaweedFS later)
   │  declares: Modules it needs  +  Effects it produces / consumes
   ▼
Module                          a runnable service (compose fragment + requires) — unchanged
Effect                          a declarative produce/consume wiring record (NEW primitive)
```

- **Capability** is the UI-facing unit. The environment stores selected *capabilities*, not
  modules. Each capability resolves to one **Provider** (a default for now; the Provider layer
  exists so swapping is a config change, not a refactor — no provider-picker UI yet).
- A **Provider** declares the **Modules** it runs and the **Effects** it produces or consumes.
- **Effect** is the new primitive: a typed produce/consume edge with a payload. The resolver
  already computes a graph; effects become payload-carrying edges in it, so resolution and the
  future visualization share one structure.

Effects implemented now (each reuses an existing code path):

| Effect | Produced by | Consumed by | Action | Status |
| --- | --- | --- | --- | --- |
| `LineageEndpoint(url)` | Lineage→Marquez (via Envoy) | Hydrofoil (in-process) | `HostConfig.lineage_endpoint` → `FlightSqlServiceImpl::with_lineage` | **wired** |
| `ObjectStorage{bucket}` | ObjectStorage→Azurite | MLflow | bucket created by the provider's init container | **wired** (MLflow bucket) |

Because the desktop runs Hydrofoil **in-process** (`desktop-host`), applying the lineage
effect is just setting a `HostConfig` field before `build()` — no container reconfiguration.

### Observability: a per-env opt-in to a shared, app-level sink

Observability is special and does **not** follow the provider/module/effect path. Two facts
shape it: telemetry is interesting *across* environments (you want one place to see engine
behavior), and OpenTelemetry initializes **once per process** (`global::set_tracer_provider`).
So:

- **The sink is a single, shared, app-level service** — one Jaeger all-in-one
  (`services/desktop/jaeger.yaml`), in its own fixed `ol-telemetry` compose project, **not**
  part of any environment's project. It is started **lazily** by the *first*
  observability-enabled environment and lives for the app's lifetime (its own `Telemetry`
  slot, separate from the per-env `Supervisor`); torn down only on app exit. An environment
  that doesn't opt in never starts it.
- **Emission is the per-environment opt-in.** `Observability` is a capability with no provider
  and no module (`Capability::is_shared_infra`); selecting it on an environment means "ensure
  the shared collector is up and the global tracer is initialized (once), so this env's engine
  spans reach Jaeger." The global tracer is initialized once, on first opt-in, via
  `desktop-host`'s re-export of `hydrofoil::telemetry::init_tracing_subscriber`, pointing at
  the shared Jaeger's OTLP/HTTP endpoint.

**Known limitation (process-global emission).** Because the engine emits through the global
`tracing` subscriber, once *any* observability-enabled environment has initialized it, the
subscriber is process-wide. A later environment that did **not** opt in would still have its
spans collected. True per-engine gating isn't achievable with a process-global subscriber;
the opt-in faithfully controls whether the collector + global init happen at all, which is the
meaningful user-facing control. Spans are tagged with `environment.id` for per-env filtering.

### Deferred (designed, not wired)

- **UC credential vending.** The `ObjectStorage` effect *carries* the credential payload
  (`endpoint`, `credentials`, `bucket`) and documents Unity Catalog as a second consumer
  (create an External Location + Storage Credential via the existing environment credential
  path from [`0015`]). The UC API call is not wired this round.
- **Provider swap / SeaweedFS.** Each capability has one default provider; the Provider layer
  exists in the model but the UI shows capabilities only. A second object-storage provider
  (SeaweedFS/S3) and a provider picker are future work — they emit the same `ObjectStorage`
  effect, so consumers (MLflow, UC) are unaffected.

## Consequences

- The UI frames choices as capabilities; the technology mapping lives in the model, where it
  can evolve without UI churn.
- Cross-service wiring is declarative and centralized: a new consumer of an existing effect
  (e.g. UC consuming `ObjectStorage`) is added without touching producers.
- The resolved graph now expresses *connectivity with payloads*, ready for visualization.
- Effects must be applied **after** their producing services are healthy (endpoints aren't
  known until then) and **before** the consumer starts — for in-process Hydrofoil that means
  resolving effects, then building `HostConfig`, then `desktop-host::build`.
- One direction of dependency only (consumers→producers) keeps the graph acyclic, consistent
  with the UC-on-host topology: Hydrofoil/UC consume; Docker services produce.
