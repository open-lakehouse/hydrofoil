# Platform-level policy architecture — design overview

> The platform companion to the three per-capability designs. Where
> `docs/policy-enforcement-design.md`, `docs/open-lineage-design.md`, and
> `docs/session-management.md` design enforcement *within one DataFusion session*,
> this document zooms out: policy as a cross-component platform concern, spanning the
> catalog, the query engine, and (future) agent tool-call APIs. It is deliberately a
> **menu of options with tradeoffs**, not a single prescription — it records where we
> are, what is missing, and the decisions we are weighing, leaning toward a hybrid PDP
> without hard-committing.

## Why

A governed lakehouse evaluates policy in *more than one place*. The catalog must
decide whether a principal may see a table or be handed a storage credential; the
query engine must gate the query and shape the rows/columns a principal sees; an agent
runtime must decide whether a tool call is allowed given what the session has already
touched. These are different enforcement points answering questions about the *same*
logical operation, often the *same* principal, frequently within the *same* session.

Today each of these lives — or will live — on its own. We have a real two-layer Cedar
stack inside the engine, a separate `Policy` trait inside the catalog, and a shared
Cedar substrate underneath both. What we lack is the platform view that ties them
together:

1. **Is there a global policy, or many local ones?** One source of truth for *rules*
   is settled (the OCI bundle). The open questions are where decisions are *made* and
   where dynamic, session-scoped state *lives*.
2. **How is a single operation correlated across hops** so that state gathered at one
   enforcement point (a credential vend, a column read, a taint) is attributable at the
   next?
3. **How does partial evaluation thread along that chain** — when no single hop has the
   full context, can a policy be progressively refined as facts accrue?
4. **Where does context come from?** Every attribute a policy reads must be sourced
   from *somewhere* — the catalog, lineage, the network edge, an accumulated facts
   store. These Policy Information Points (PIPs) are first-class, and their completeness
   is exactly what determines full decisions vs. residuals.
5. **How do column taints from lineage feed back into decisions**, including the
   agentic case where a session that has consumed regulated data must be constrained on
   downstream tool calls?

The thesis of this document: **one policy source of truth, many enforcement points,
correlated by a propagated session/trace context, fed by many information points, with
dynamic taint state that flows back into decisions.**

## Diagram

The PEP / PIP / PDP topology is drawn in [`policy-architecture.d2`](policy-architecture.d2)
(rendered: [`policy-architecture.svg`](policy-architecture.svg)) — the PAP/OCI bundle, the
embedded Cedar PDP, the four PEPs along the catalog → engine → agent-tool chain (correlated
by session id), the PIPs that feed facts, and the taint-ledger feedback loop. Regenerate with
`d2 --layout elk docs/policy-architecture.d2 docs/policy-architecture.svg`. Solid = built;
dashed = designed/future.

## Design principles

These continue the principles the sibling docs established — one platform, one set of
rules.

1. **Plug in, don't fork.** Reuse the extension seams and the Cedar substrate we
   already link; add platform structure around them, not a parallel engine.
2. **One policy language, one source of truth; many information points.** Author in
   Cedar, distribute one OCI bundle of *rules*. The *inputs* to those rules are gathered
   from many PIPs — keep rules and facts cleanly separated.
3. **Fail closed, everywhere, including distributed.** A policy or a PIP that can't be
   reached denies (or masks). A central component's outage must never silently open the
   platform.
4. **Pluggable at the edges, opinionated in the middle.** The PIP sources, the
   transport, and the residual translation are traits; the decision envelope and the
   layering are fixed and correct.
5. **Learn from prior art, defer the rest.** Adopt the models NIST, Cedar, OPA, Atlas,
   Unity Catalog, and the agentic-AI literature have validated; defer the machinery that
   doesn't earn its keep in a first version, behind named seams.

## Where we are today, precisely

The platform already has two enforcement points and a shared substrate beneath them.

**Catalog PEP (`unitycatalog-rs`).** `crates/server/src/policy/mod.rs` defines a
`Policy<Cx>` trait — `authorize(resource, permission, context)` and
`authorize_many(...)` returning `Decision` (`Allow`/`Deny`) over a `Principal`
(`Anonymous` / `User`) and a `Permission` enum (`Read`, `Write`, `Manage`, `Create`,
`Use`, `Browse`, `Select`). The shipped impl is `ConstantPolicy`. Credential vending
(`crates/server/src/api/temporary_credentials.rs`) gates on the operation —
`required_permission(VendOperation::Read) → Permission::Read`,
`ReadWrite → Permission::Write` — via `authorize_checked` before any credential is
minted. The upstream-proxy decorator (`handlers/upstream.rs`) checks policy *then*
filters list responses so callers never see resources they lack rights to. Identity is
established by an `Authenticator` in `rest/auth.rs`.

**Query-engine PEP (`datafusion-cedar` + `hydrofoil`).** `src/policy.rs` defines the
engine `Policy` trait: `is_allowed(logical_plan, principal) → Decision` (Layer 1, the
coarse access gate, run *after* optimization) and `table_policy(table, schema,
principal) → TablePolicy` (Layer 2, fine-grained governance, run *before*
optimization). `TablePolicy { row_filters: Vec<Expr>, column_masks: HashMap<String,
Expr> }` (`src/govern.rs`) is injected by `govern_plan` / `GovernRewriter` as `Filter`
and `Projection` nodes. The fine-grained predicates come from Cedar **partial-evaluation
residuals**, translated to DataFusion `Expr` behind the `ResidualTranslator` trait
(`src/translate.rs`, impl `CedarResidualTranslator::to_predicate`). The session wires
it in `LakehouseSession::create_physical_plan` (`hydrofoil/src/session.rs`); the
principal is parsed in `hydrofoil/src/identity.rs::principal_from_metadata`.

**Shared substrate.** Both PEPs sit on Cedar via `cedar-local-agent`'s
`Authorizer<P, E>` and the `SimplePolicySetProvider` / `SimpleEntityProvider` traits.
`cedar-oci::OciPolicyProvider` (`crates/cedar-oci/src/oci/mod.rs`,
`from_reference(...)`) implements both, sourcing one OCI bundle = policy set + schema +
entities under custom media types; the engine builds its authorizer via
`CedarPolicy::from_oci` (`crates/datafusion-cedar/src/cedar.rs`).

**Lineage substrate.** `crates/open-lineage` walks the optimized plan and emits
column-level lineage. The `Transformation` facet carries a `masking: bool`
(`src/facets.rs:302`) that is currently hardcoded `false` (`src/extract.rs:117,188`) —
a deliberately-dangling thread this design picks up.

**Known gaps already tracked.** The principal/session `"key"` singleton and
execution-time COMPLETE/FAIL correlation are documented in `session-management.md`.

**The divergence to note.** There are two `Policy` traits over the same Cedar engine:
resource-level `(ResourceIdent, Permission)` in the catalog, plan-level `LogicalPlan`
in the engine. That divergence is the seam where unification is possible (Critical
decision 2).

## Standards & prior-art alignment

We ground the platform in established models so the choices below read as deliberate.
Each point states the source and our stance; the full mapping of *which gaps we cover*
is in the Gap analysis section.

**NIST SP 800-207 / 207A (zero trust).** 800-207 splits the PDP into a **Policy Engine
(PE)** that renders the verdict and a **Policy Administrator (PA)** that *executes* it
and mints the session credential. This maps cleanly onto us: our embedded Cedar is the
PE; **the catalog's credential vending is the PA** — it should act only on a PE verdict.
800-207A's cloud-native refinement gives us three more ideas: layered, additive PEPs
(catalog → engine → agent) and *"carry the end-user authorization across hops"* — the
canonical justification for our multi-PEP + propagated-session design; an explicit
**PAP / control plane** that versions and fast-pushes the policy bundle to every PEP;
and **workload identity** (SPIFFE/SVID-style) for the engine↔catalog and agent↔engine
hops, not just user identity. 800-207 also expects a *Trust Algorithm* fed by many
signal sources (threat-intel, SIEM, device posture) — a class of PIP we do not yet have
(Critical decision 4).

**Cedar (OOPSLA 2024).** We inherit deny-by-default and forbid-overrides — `forbid` is
the right place for hard taint guardrails — and schema-validated soundness. The net-new
opportunity is Cedar's **SMT-based analyzability**: use it as a CI release gate on the
OCI bundle, proving invariants ("no policy ever unmasks a column tagged `pii` for role
`analyst`") and bundle-to-bundle refinement before promoting a version. This requires
modeling taints and tags as **typed Cedar schema entities/attributes**, not free-form
context, to preserve the soundness guarantee.

**OpenID AuthZEN, Trino-OPA, OPA Compile / UCAST.** These define the PDP↔PEP contract
and the partial-evaluation contract we should *speak rather than reinvent* — folded into
Critical decisions 2 and 4.

## Critical decision 1 — PDP topology

The central question: where are decisions *made*, and where does dynamic state *live*?
Vocabulary (XACML / NIST 800-207): the **PDP** decides — split into PE (verdict) + PA
(execution/credentialing); the **PEP** enforces; the **PAP** authors and distributes
rules; the **PIP** supplies attributes.

**A. Fully embedded / library PDP.** Every PEP links the Cedar engine and sources the
same OCI bundle. *Pro:* lowest latency, no new service, exactly today's code, no hop in
the query hot path. *Con:* no central audit, no shared dynamic state — and crucially,
**taint accumulation has nowhere to live**; each PEP re-derives context independently.

**B. Standalone policy server (remote PDP).** A new service exposes
`evaluate` / `evaluate_partial`; PEPs are thin clients. *Pro:* central audit, a single
owner of dynamic state, one upgrade surface. *Con:* a network hop on *every* scan
authorization, a new service to build and operate, and availability becomes
critical-path — fail-closed turns a PDP outage into a platform outage.

**C. Hybrid — embedded hot path + central session-state PDP (leaning).** The coarse gate
and partial evaluation stay embedded (latency-sensitive, principal-local); a central
server owns *session-scoped dynamic state* (the taint ledger, the agent tool-call
history) and audit, consulted only at session/operation boundaries — not per scan.
*Pro:* keeps the query fast path local while giving the agentic story a home for
cross-call state. *Con:* two code paths; we must define precisely which decisions are
local vs. central and the state-sync semantics between them.

```
        ┌─────────────────────────── PAP / control plane ───────────────────────────┐
        │   author Cedar → build OCI bundle (policyset + schema + entities) → push   │
        └───────────────┬───────────────────────┬───────────────────────┬───────────┘
                        ▼                         ▼                       ▼
   ┌── embedded PE ──┐         ┌── embedded PE ──┐        ┌── embedded PE ──┐
   │  Catalog PEP    │         │  Engine PEP     │        │  Agent-tool PEP │
   │  (+ PA: vend)   │         │  (gate+govern)  │        │                 │
   └────────┬────────┘         └────────┬────────┘        └────────┬────────┘
            └──── consult at operation boundaries ─────────────────┘
                                  ▼
                  ┌───────────────────────────────┐
                  │  Central session-state PDP     │   ← taint ledger, audit,
                  │  (dynamic facts, keyed by      │     session-scoped facts (1C)
                  │   correlation id)              │
                  └───────────────────────────────┘
```

**Leaning: C.** **A** wins if we never need cross-call dynamic state (no agentic story,
no session taints). **B** wins if central audit / regulatory logging dominates and the
per-scan hop is acceptable. Note that *policy distribution* is OCI/GitOps in all three —
only the *decision locus* and the *dynamic-state locus* differ.

## Critical decision 2 — Unify the two `Policy` traits, or keep them distinct

**A. Keep distinct.** Catalog stays `Policy<Cx>` over `(ResourceIdent, Permission)`;
engine stays over `LogicalPlan`. *Pro:* each is shaped to its enforcement site; zero
churn. *Con:* two mental models, divergent request construction, duplicated
principal/context plumbing — and a third (agent) PEP would add a third shape.

**B. Shared decision envelope, thin adapters (leaning).** Define one
`PolicyRequest { principal, action, resource, context }` ↔ `Decision` / `PartialResponse`
envelope — essentially the Cedar `Request` shape — that both traits lower into. Each PEP
keeps a thin adapter: the engine already has `authorize_plan` (`LogicalPlan` → requests);
the catalog's `SecuredAction` → request is a small map. *Pro:* one audit format, one
place to attach session/taint context, reuse across the future agent PEP. *Con:* an
abstraction to design and hold stable — the highest-design-cost item here.

**Concretely, adopt the OpenID AuthZEN Access-Evaluation shape for the envelope** —
request `{ subject, action, resource, context }`, response
`{ decision, context: { obligations, reasons } }`. It is a near-1:1 map to Cedar's
`(principal, action, resource, context)` and gives us a standard, batchable
(`deny_on_first_deny`) contract instead of a bespoke one, with the side benefit that
AuthZEN flags policy-reconnaissance/DoS against the PDP and prescribes authenticating
PEPs to it.

The decisive payoff is **obligations**. Model row filters, column masks, and
tool-payload redaction as AuthZEN `obligations` — *permit-with-obligation* — rather than
as a separate governance path. One envelope then carries fine-grained enforcement at the
engine *and* output-side controls at the agent PEP. Also adopt Trino-OPA's **batched,
index-keyed** filter/mask contract (one evaluation across many columns/objects) and its
optional **`identity` "evaluate-as" override** (mask-as-if-role-X) for impersonation
governance.

**Leaning: B**, because it is what makes the hybrid PDP (1C) and the agent PEP tractable.
**A** is fine if we are certain we will never add a third PEP.

## Critical decision 3 — Cross-component correlation

A single logical operation — an analyst query, or an agent turn that hits the catalog,
runs a query, then calls a tool — spans multiple services and RPCs. Session-scoped state
(taints, prior decisions) needs a **stable correlation key** to be attributable. This
section deliberately keeps all options open.

**A. Reuse OpenLineage run / parent-run context.** The `parent.run.runId` / `root.run`
facets already propagate via `x-openlineage-parent-*` headers (`hydrofoil/src/lineage.rs`).
*Pro:* zero new propagation surface; the policy session *is* the lineage run, so
taint-from-lineage is naturally co-keyed. *Con:* couples policy lifetime to lineage
semantics; lineage run ids are per-job, not obviously per-agent-session; lineage is
best-effort and swallows errors, whereas policy must not.

**B. Dedicated policy session/trace id (W3C `traceparent`-style).** A distinct id minted
at session establishment, propagated on its own header across catalog, engine, and agent
APIs. *Pro:* clean separation, policy-owned lifetime, aligns with any OTel trace context
the platform already carries. *Con:* new plumbing in every PEP and client; must be
reconciled with lineage run ids to join taints.

**C. Reuse the Flight SQL / catalog session identity** from `session-management.md`
(protocol cookie / handshake). *Pro:* builds on already-designed work. *Con:* covers only
the engine/catalog hop, not arbitrary agent tool calls — needs a bridge to the agent
runtime.

| Option | Propagation surface | Lifetime semantics | Taint-join cost | Agent-hop coverage |
| --- | --- | --- | --- | --- |
| A — lineage run | none new (reuse headers) | per-job (lineage) | free (same key) | weak (job-shaped) |
| B — policy trace id | new header, all PEPs | policy-owned | needs reconcile w/ run id | strong |
| C — session id | partial (FlightSQL) | per-connection | needs bridge | none without bridge |

No verdict. Decide this **alongside** the session-management rework rather than
independently. The most general shape is **B layered over the session-management
SessionId**; **A** is the pragmatic shortcut for an early demo where policy and lineage
already share a run.

Whatever the key, it is *transport, not trust* — see Critical decision 6 on binding it
cryptographically.

## Critical decision 4 — Policy Information Points: where context comes from

Cedar's `Request` is `(principal, action, resource, context)` plus an `Entities` store.
*Every* attribute a policy reads — `principal.role`, `resource.region`, `resource.tags`,
`context.observed_taints`, `context.network.client_ip` — must be sourced from a PIP.
The framing: **policies are static; their inputs are gathered from many PIPs, and the
completeness of those inputs is exactly what determines a full decision vs. a residual.**

### The PIP catalogue

**Catalog metadata (UC-RS) — the authoritative `resource.*` PIP.** Table/schema/catalog
identity, ownership, grants, **classification tags / column tags**, external-location
bindings. Lives in the catalog's Postgres store. Wires in as Cedar **resource entities +
attributes** and as the `@target_tag` / `@applies_to_tag` scoping (Critical decision 5).
The open question is **static bundle vs. live lookup**: (a) bake catalog facts into the
OCI bundle at publish time — consistent and versioned, but can be stale; (b) query the
catalog at decision time — fresh, but adds a hop and an availability dependency;
(c) cache-with-TTL hybrid.

**Lineage system — the derived/dynamic resource PIP.** Column-level lineage and the
accumulated **taints** (Critical decision 5). Supplies `resource.derived_taints` (static,
per dataset) and feeds the session ledger (`context.observed_taints`, dynamic). Lives in
whatever consumes OpenLineage events — Marquez/DataHub or our own taint store. The
`masking` flag is the declassification signal flowing back in.

**Principal/identity PIP.** `principal.*` attributes — role, region, group membership,
agent-vs-human. **Implemented (v1):** the `IdentityProvider` trait
(`datafusion-cedar`) enriches the authenticated principal with attributes **and group
membership**, folded into Cedar as the principal entity's parents + a transitive
group-entity closure (`PrincipalIdentity::to_entities`). This makes membership a
*dynamic* request-time fact rather than a static OCI-bundle entity — the
**static-bundle→dynamic-membership shift**. The v1 `ConfigIdentityProvider`
(`hydrofoil/src/identity.rs`) sources facts from a config map (moving what the bundle
hardcoded behind the seam); a real IdP/directory backend (OIDC userinfo / SCIM / LDAP) is
the same trait. Enrichment is resolved once per session, cached on the `Engine`, and is
fail-closed; the cache placement lets it move to a per-query TTL lookup later. Header
`role`/`region` remain transport (advisory), not trust; group membership comes only from the
provider. See `docs/adr/0008-principal-identity-resolution.md`. The `principal_from_metadata`
headers are still the pre-auth transport seam; a real authenticator / interceptor upstream
remains future work.

**Network / request-context PIP (Envoy / proxy-injected).** Facts injected at the network
edge — client IP, mTLS subject, geo, request headers, rate/quota signals — surfaced via an
Envoy `ext_authz` filter or proxy. Two shapes: (a) Envoy `ext_authz` → central PDP *at the
edge* as a coarse pre-filter before the request reaches the service — this is NIST
800-207A's **sidecar-as-universal-PEP / security-kernel** model, so we can *promote* Envoy
from a pure PIP to a real coarse enforcement layer; (b) the proxy *injects headers* that
downstream PEPs fold into their Cedar `context`. Edge-injected facts are trusted *because*
the edge is authenticated infrastructure — unlike client-asserted headers.

**Security-signal PIPs (gap flagged by NIST 800-207).** Threat-intel, SIEM, device
posture, behavioral signals — core trust-algorithm inputs in 800-207, absent from our
governance-centric set. Noted here as a deliberately-deferred PIP class that would plug
into the request `context`, so the omission is explicit rather than silent.

**Session-state / facts store PIP — the accumulated-facts datastore.** A store, keyed by
the correlation id (Critical decision 3), of facts gathered *during* a session/operation:
observed taints, prior decisions, consent flags, step-up-auth results, tool-call history.
This is the dynamic PIP the central session-state PDP (1C) owns. Backing options:
in-memory per-process (simple, not shared) vs. Redis/shared KV (shared, eventual) vs.
durable (audit-grade, slower). **This store is what makes "policies dynamically evolve
based on context" real** — see below.

**PAP / policy bundle (`cedar-oci` OCI).** For completeness: the *policy set + schema* is
distributed separately from the *facts*. One source of truth for rules; many PIPs for
inputs.

### How PIPs interact with partial evaluation — the key dynamic

We rarely have all context at once. A PEP evaluates with the facts it has; missing PIP
inputs are left **unknown**, so `is_authorized_partial` returns a **residual** — a reduced
policy that still mentions the not-yet-known attributes. As later hops gather more facts,
each **refines the residual** until it collapses to a concrete allow/deny — or to a row
filter / column mask, which is just a residual we *render into the plan* instead of
deciding now. The residual *is* the carrier of "policy evolves as context arrives," and
the session-facts store is what accumulates the inputs that drive the collapse.

```
 policy + partial context ──is_authorized_partial──▶ residual ──refine──▶ … ──▶ decision

  edge (network ctx)   →   catalog (resource bound)   →   engine (columns/taints bound)   →   agent (tool target + ledger bound)
   residual r0                  residual r1                    residual r2 (= row filter / mask)        r3 → Allow / Deny / Obligation
```

This is exactly OPA's **Compile API** (unknowns in → residual out → push down) and Cedar's
`is_authorized_partial`. Adopt three things from them:

1. A **dialect-neutral residual AST** (à la OPA **UCAST**) so the *same* residual can lower
   to a DataFusion `Expr`, the agent-tool PEP, or a future engine — don't couple residual
   lowering to DataFusion. Our `ResidualTranslator` seam is the right place.
2. An explicit **logical→physical mapping layer** (OPA's `targetSQLTableMappings`) from
   Cedar resource/attribute unknowns to physical Delta columns.
3. The **`[[]]` = always-true / absent = deny** trichotomy for the no-filter / filter /
   deny-entirely residual cases.

Mind the Cedar-specific sharp edge: to preserve error semantics the evaluator does *not*
constant-fold aggressively, so residuals keep `true && unknown`-shaped structure that the
lowering code must handle. The recent "harden governance rewriter" work is the start of
getting this right.

Critical decision 6 supplies the agent-hop *mechanism* for this; this section supplies the
*why*.

## Critical decision 5 — Taint model & lineage→policy feedback

The platform's distinctive bet, designed here conceptually.

**Taint origin & tag-scoped policy — align with Unity Catalog ABAC.** Columns and datasets
carry classification **tags** (PII, regulated, region-bound). Tags are PIP attributes on
Cedar resource entities; the catalog already models them, the OCI entity bundle is the
natural carrier, and Policast's `@target_tag` / `@applies_to_tag` is the in-house prior
art. Unity Catalog's ABAC validates the shape almost 1:1 — `@applies_to_tag` ≈ UC
`MATCH COLUMNS has_tag(...)`, `@filter_type(row_filter|column_mask)` ≈ UC `ROW FILTER` /
`COLUMN MASK`, principal targeting ≈ `TO / EXCEPT`. Borrow three things from UC:

- **Tag-matched column *alias* indirection** (`MATCH COLUMNS … AS alias` / `USING
  COLUMNS`) so a tag-scoped residual binds columns by tag/alias, not physical name — one
  policy portable across many tables.
- **`EXCEPT principal`** carve-outs.
- **Tag assignment as a permissioned catalog op** — who may apply which tag is itself
  gated.

Our advantage: Cedar's `@deny_override` plus analyzability gives us the **multi-policy
precedence/combination rule UC leaves unspecified** (what wins when two masks match one
column) — we must define it explicitly. UC's **auto data classification** (LLM/pattern +
human-in-the-loop) is a plausible future feeder of the tag layer.

**Taint propagation via lineage — model it on Apache Atlas.** The `open-lineage`
extraction already computes input-field → output-field edges per `Projection` /
`TableScan`. The propagation rule: an output column inherits the **union** of its input
columns' taints. Lift Atlas's three-tier propagation control to column granularity:

1. A per-tag **`propagate` flag** — a classification can be policy-relevant on a table yet
   *not* taint derived columns.
2. Per-edge **enable/disable** — a topology-level cut.
3. A per-edge **blocked-propagated-classifications list** — our **declassification
   primitive**: masking / hashing / aggregation emits an explicit blocked-tag set on the
   lineage edge, rather than relying only on implicit inference.

Describe the taint lattice as set-union of tags, with mask/hash/aggregate as
lattice-lowering operations. Adopt Atlas's **union-with-reference-counting +
re-evaluation on change**: when a table is dropped or a lineage edge changes, recompute,
and drop a taint only when no surviving path justifies it. (This is Atlas's documented
bug-magnet — design recomputation in from day one; under-counting silently declassifies.)

Keep **lineage-propagated taints distinct from containment-inherited tags**: a tag
inherited down the catalog hierarchy (UC-style parent→child) is a *static resource
attribute*; a tag propagated down the lineage graph is a *computed taint*. They have
different lifecycles and declassification rules. Propagation is knowable at write/CTAS time
(lineage is known when the output is produced); state whether it is computed inline in
extraction or by a downstream consumer of lineage events.

**Wiring the `masking` flag — the first concrete lineage↔policy integration.** Once
`govern_plan` injects mask `Projection`s, extraction should set `masking: true`
(`extract.rs`, today hardcoded `false`). That flag both reports to observability *and*
signals declassification to the taint engine — the same `Projection` that masks a column
is the edge that blocks its taint from propagating.

**Feedback into decisions — two directions.**

- *Static.* Accumulated dataset/column taints become resource attributes the gate and
  governance policies condition on — closing the loop OCI-entities → Cedar.
- *Dynamic / session-scoped.* A **session taint ledger**: as a session reads tainted
  columns, the set of taints it has *observed* accumulates. This ledger is the
  session-scoped state that lives in the central PDP (1C), monotonic per session, keyed by
  the correlation id (Critical decision 3). It is one entry in the session-facts store PIP
  (Critical decision 4) — taints are one fact-class among consent flags, step-up results,
  and tool-call history.

## Critical decision 6 — Agentic authorization on the session taint ledger

Designed conceptually, no demo. This is where the agentic-AI literature
(Authorization-Propagation in Multi-Agent AI, OIDC-A, OWASP Top-10 Agentic, the LLM-agent
privacy review, and Bauplan) maps most directly.

**The problem.** An agent session invokes tools; some tools (export, send-email,
call-external-API) must be **denied or constrained** if the session has consumed data of
certain classifications — *"you read regulated PII this turn, so `send_external` is now
blocked, or requires the data be masked first."* This is precisely OWASP **ASI01 goal
hijack / EchoLeak-style silent exfiltration** and the **aggregation-inference** problem
named in the authorization-propagation paper. Our taint ledger is data-flow control that
blocks the exfiltration *action regardless of how the prompt was hijacked* — our single
strongest property, and the headline of the agentic story.

**The mechanism.** Each tool call is a new PEP: a Cedar `Request` whose `context` includes
the session's current taint ledger (`context.observed_taints`). Policies express
constraints like `forbid send_external when context.observed_taints.contains("PII")`. The
PDP that evaluates this is the central session-state PDP (1C), because it owns the ledger.
**Output-side control via obligations:** beyond allow/deny, a decision can carry an AuthZEN
**obligation to redact/mask the tool payload** (reusing the column-mask machinery) — this
closes the privacy review's "you gate the action but don't sanitize the payload" gap.

**Agent identity & delegation (OIDC-A) — currently a gap, fold in.** Cedar requests carry a
principal but no *agent* identity. Add OIDC-A claims as Cedar context/entity attributes:
`agent_type`, `agent_model`, `agent_instance`, `delegator_sub`, `delegation_chain`,
`agent_capabilities`. This lets the tool PEP distinguish a rogue or unexpected agent
reusing a human's authority (OWASP ASI03/ASI10) and makes hop-by-hop partial evaluation
operate on **authority**, not just taints. Pair it with the **task-scoped authorization
envelope** from the propagation paper — permitted actions + resource bounds + delegation
depth + TTL, non-escapable, authority that *narrows* per hop — as the formal frame for our
progressive refinement; adopt its monotonic-narrowing invariant.

**Extend the ledger on two axes.** (i) **Taint on ingress** — tool/RAG outputs that return
sensitive data must *add* to the ledger, not only catalog reads (the privacy review's
tool-use leakage path). (ii) **Integrity taints** — track not just confidentiality ("is
this PII") but provenance trust, to begin addressing memory/context poisoning (OWASP
ASI06).

**Bind the correlation id cryptographically.** Our session/trace id (Critical decision 3)
is spoofable as plain transport; an attacker could forge the id that gates tool calls.
Require PoP / mTLS-bound tokens (OIDC-A / RFC 8705) so the ledger is attributable. This is
the same trust-boundary caveat as `principal_from_metadata`, raised to the platform level
(OWASP ASI07; AuthZEN PEP-authentication).

**Partial evaluation along the chain — option vs. simpler default.** A *global* policy can
be **partially evaluated at each hop** as the request becomes concrete: the catalog
evaluates with principal+resource known but session-taints unknown (residual), the engine
refines it as columns are read, the agent runtime closes it at tool-call time. This is the
agent-hop instance of the progressive-residual model of Critical decision 4 — the residual
carried from earlier hops collapses once the tool target and the ledger are bound. Weigh it
against the far simpler **re-evaluate fully at each PEP with the accumulated context from
the facts store**: the latter is almost certainly the right v1; progressive residual
refinement is the elegant end-state. Make the tradeoff explicit and pick per-phase.

## Critical decision 7 — Write-path & the credential-vending bypass

Two prior-art findings converge on the write/credential boundary.

**The vended-credential bypass (Iceberg-REST critique + ecosystem).** Our two-layer split —
catalog gates and vends; engine enforces RLS/CM — is exactly the answer the Iceberg-REST
ecosystem prescribes, which *validates the design*. But the guarantee holds **only for data
that flows through hydrofoil.** The moment the catalog vends a storage credential to any
*other* engine — Spark, Trino, a raw-S3 notebook — our Cedar row-filter/column-mask
residuals are bypassed and that engine reads raw files. This is the platform's sharpest
limitation. Mitigations: scope vended credentials as tightly as possible (path/expiry);
and/or treat any table carrying masking/row-filter tags as **non-vendable to coarse-only
consumers** — the catalog PA must *deny* a raw credential for a governed table and force the
query through the enforcing engine.

**Commit-time enforcement & isolation (Bauplan).** Our enforcement is at *read/query* and
*tool-call* time, not bound to *write/commit* boundaries. Bauplan argues governance should
attach to transactional commit boundaries, with per-agent **branch isolation** and
**self-healing rollback** on violation. A future direction for write-capable agents: bind
policy + lineage to commit time, give agents an isolated (copy-on-write) branch so
violations roll back rather than leak. Complementary and deferred — our axis is
confidentiality/taint; Bauplan's is transactional integrity/isolation; they compose.

**Policy-bundle freshness (Iceberg-REST critique, generalized).** The critique's real lesson
for us: our OCI bundle distribution has no freshness/version-staleness contract, so a PEP
could enforce a *stale* bundle. Specify a freshness / version-pinning / propagation-SLA
contract for bundle distribution — this is the PAP/control-plane idea from the standards
section and NIST 800-207A's sub-second push requirement, made concrete.

## Gap analysis — what the literature raises, and our coverage

✅ addresses · 🟡 partially addresses · ❌ does not address.

| Gap | Source(s) | Coverage | Where |
| --- | --- | --- | --- |
| **Aggregation-inference / EchoLeak exfiltration** | Authz-Propagation; OWASP ASI01; privacy review | ✅ | taint ledger + tool PEP (d5/d6) — *headline strength* |
| **Vended-credential FGAC bypass** | Iceberg-REST critique | ✅ | two-layer PEP split + non-vendable-governed-table guard (d7) |
| **Provenance/lineage as governance substrate** | Authz-Propagation; Bauplan; Atlas | ✅ | OpenLineage column lineage + taint propagation (d5) |
| **Row filters / column masks from partial eval** | OPA Compile; Trino-OPA; Cedar PE | ✅ | residual → `Expr` (existing); UCAST/mapping refinements (d4) |
| **PE/PA separation, layered PEPs, carry authz across hops** | NIST 800-207/207A | ✅ | catalog = PA; multi-PEP + correlation (standards, d1, d3) |
| **Output-side sanitization of payloads** | privacy review | 🟡 | AuthZEN obligations (d2/d6) — new work |
| **Trust-algorithm richness (contextual/behavioral, scored)** | NIST 800-207 | 🟡 | only the taint ledger is contextual; criteria-based + singular otherwise |
| **Cedar analyzability as a release gate** | Cedar OOPSLA | 🟡 | available, unused (standards, d2) |
| **Bundle freshness / fast policy push / explicit PAP** | Iceberg-REST; NIST 207A | 🟡 | named, not built (d7, standards) |
| **Multi-policy precedence/combination rule** | UC ABAC (silent) | 🟡 | `@deny_override` partial; rule undefined (d5) |
| **Agent identity & delegation chain / transitive accountability** | OIDC-A; Authz-Propagation | ❌ | d6 future |
| **Cryptographic binding of correlation id** | OWASP ASI07; AuthZEN | ❌ | d6 future |
| **Workload identity (SPIFFE/SVID)** | NIST 800-207A | ❌ | not modeled |
| **Security-signal PIPs (threat-intel/SIEM/device posture)** | NIST 800-207 | ❌ | deferred PIP class (d4) |
| **Supply-chain / rogue-agent containment; integrity taints** | OWASP ASI04/06/10 | ❌ | d6 future (integrity-taint sketch only) |
| **Commit-time enforcement, branch isolation, self-healing rollback** | Bauplan | ❌ | d7 deferred |

## Missing components

Design-only here; each tagged with the decision that addresses it.

- Shared decision envelope (d2) and its AuthZEN shape + obligations (d2).
- Correlation-id propagation (d3).
- PIP wiring per source — catalog-attribute provider, lineage/taint provider,
  network/Envoy `ext_authz`, session-facts store (d4).
- Neutral residual AST / UCAST + logical→physical mapping layer (d4).
- Session-state / central PDP service (d1, d4).
- Taint engine / lineage consumer (d5) and `masking`-flag wiring (d5).
- Agent-tool PEP (d6); agent identity + delegation chain + PoP-bound correlation id (d6).
- PAP / control plane for versioned bundle distribution + freshness contract (d7, standards).
- Non-vendable-governed-table guard on credential vending (d7).
- Cedar SMT invariant/refinement CI gate (standards).
- Central audit sink (d1).

## Risks

- **Fail-closed across a distributed PDP.** A central-server outage must not silently open
  *or* hard-down the platform; define a per-PEP fallback posture.
- **Correlation-id spoofing.** The id is transport, not trust; it must ride authenticated
  context (and be PoP-bound for the agent PEP — d6).
- **Governance-bypass via vended credentials (d7).** The platform's strongest
  confidentiality guarantee evaporates if a raw storage credential reaches a non-enforcing
  engine. A correctness risk, not just a hardening item.
- **Taint over-tainting.** A monotonic ledger that never clears locks a session down
  uselessly; needs declassification (d5) and session-scoping discipline.
- **Taint recomputation correctness (d5, per Atlas).** Multi-path reference-counting +
  re-eval on drop/edge-change is the documented bug-magnet; under-counting silently
  declassifies.
- **Stale vs. live PIP facts (d4).** Baked-in catalog facts can be stale; live lookup adds a
  hop and an availability dependency. State a per-PIP freshness/consistency posture.
- **Residual leakage.** A residual handed across hops encodes the *remaining* policy
  condition; ensure it doesn't leak policy structure to an untrusted hop. AuthZEN also flags
  policy-reconnaissance / DoS against the PDP — authenticate PEPs to it.
- **`partial-eval` maturity.** Already flagged in the engine design; it compounds if used
  for chain-wide progressive refinement.
- **Coupling policy lifetime to lineage (d3A)** if lineage is best-effort.

## Phased implementation (design-only roadmap)

- **Phase 0 — naming & envelope.** Land the shared `PolicyRequest` / decision envelope
  concept (d2B, AuthZEN-shaped) on paper; pick the correlation mechanism (d3) *with* the
  session-management rework. Nothing stays embedded vs. central yet — this is vocabulary.
- **Phase 1 — PIP providers + close the lineage↔policy loop locally.** Define the per-source
  PIP provider seam (d4: catalog-attribute, lineage/taint, network/edge, session-facts);
  wire the `masking` flag; compute static dataset/column taints from lineage as Cedar
  resource attributes. Entirely within the existing embedded engine — no new service.
- **Phase 2 — session-facts store + central session-state PDP (d1C/d4).** Holds the taint
  ledger; consulted at operation boundaries, not per scan. Add the Envoy/edge network PIP.
  The embedded/central line becomes explicit here: hot-path gate + partial eval stay
  embedded; dynamic session state goes central.
- **Phase 3 — agent-tool PEP (d6).** Consumes the facts store; choose re-evaluate-fully (v1)
  vs. progressive residual refinement. Layer in agent identity, delegation chain, and
  PoP-bound correlation as they mature.

Each phase states what stays embedded vs. central, so the hybrid line of Critical decision 1
is never implicit.

## References

Publications informing this design (from `links.md`). Where a primary PDF was not directly
machine-readable, the load-bearing claims were cross-checked against authoritative secondary
sources or open-access versions; those are flagged.

- NIST SP 800-207, *Zero Trust Architecture* — PE/PA split, trust algorithm, data sources.
  (PDF read via authoritative secondary summaries; PE/PA roles and data-source list
  cross-checked.)
- NIST SP 800-207A, *ZTA for Multi-Cloud / cloud-native* — layered PEPs, sidecar-as-PEP,
  workload identity, carry-authz-across-hops, sub-second policy push.
- *Cedar: A New Language for Authorization* (OOPSLA 2024) — PARC model, deny-by-default,
  forbid-overrides, schema validation, SMT analyzability. (ACM PDF cross-checked against the
  arXiv version.)
- OpenID AuthZEN Working Group — Access-Evaluation request/response, obligations, batch,
  PEP authentication.
- OPA Compile API / partial evaluation, and UCAST — unknowns → residual, neutral conditions
  AST, `targetSQLTableMappings`, `[[]]`/absent trichotomy.
- Cedar partial evaluation (cedarland.blog) — `is_authorized_partial`, residual shapes,
  no-constant-fold caveat.
- OPA for Trino — row-filter / column-mask request/response contract, batched index-keyed
  masking, `identity` evaluate-as override.
- *Authorization Propagation in Multi-Agent AI Systems* (arXiv 2605.05440) — aggregation
  inference, task-scoped authorization envelopes, transitive accountability. (Read primarily
  from abstract + PDF; the seven-requirement names should be re-verified against the source
  before implementation.)
- *OIDC-A: OpenID Connect for Agents* (arXiv 2509.25974) — agent identity claims, delegation
  chain, PoP/attestation. (Exact claim names to be re-verified against the OIDC-A spec.)
- *OWASP Top 10 for Agentic Applications* (Dec 2025) — ASI01–ASI10 threat taxonomy.
- *On Protecting the Data Privacy of LLMs and LLM Agents* (ScienceDirect S2667295225000042 /
  open-access arXiv 2403.05156) — leakage paths, taint/flow tracking, output filtering.
  (Original ScienceDirect URL was unavailable; read via the open-access twin.)
- *Trustworthy AI in the Agentic Lakehouse* / Bauplan (arXiv 2511.16402) — governance as
  transactions, commit-time enforcement, branch isolation, self-healing rollback.
- Apache Atlas Classification Propagation — per-tag `propagate` flag, per-edge enable/disable,
  blocked-propagated-classifications list, union-with-reference-counting re-evaluation.
- Unity Catalog ABAC — governed tags, `has_tag`/`has_tag_value`, `MATCH COLUMNS … AS alias`,
  `TO/EXCEPT`, auto data classification.
- *A Critique of the Iceberg REST Catalog* (Data Engineering Weekly) + ecosystem — the
  vended-credential FGAC bypass, the two-layer enforcement answer, freshness/SLA gaps.
