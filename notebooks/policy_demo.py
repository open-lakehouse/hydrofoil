# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "adbc-driver-flightsql>=1.9.0",
#     "pyarrow",
#     "marimo",
# ]
# ///

# Cedar policy enforcement, end to end, over hydrofoil's Flight SQL endpoint.
#
# Hydrofoil applies two layers of Cedar policy to every query (see
# docs/policy-enforcement-design.md):
#   Layer 1 — a coarse access gate (allow/deny the whole query), and
#   Layer 2 — fine-grained governance (per-principal row filters + column masks),
#             active only when the server is built with `--features governance`.
#
# The principal and its attributes are carried as gRPC request metadata
# (x-hydrofoil-principal / -role / -region), parsed by
# crates/hydrofoil/src/identity.rs. This notebook issues the *same* SELECT as two
# principals and shows the gate and governance differences.
#
# ── Prerequisites ───────────────────────────────────────────────────────────
# 1. A local OCI registry (zot) with the demo policy pushed:
#        just env-up-full            # brings up zot on :10100 (among others)
#        just push-demo-policy       # pushes config/policies/demo.cedar
# 2. Hydrofoil running with the policy wired in and governance enabled:
#        HYDROFOIL_POLICY_REF=localhost:10100/hydrofoil/demo-policy:latest \
#          cargo run --features governance --bin hydrofoil
#    (or `just hydro-full` after exporting HYDROFOIL_POLICY_REF; add
#     `--features governance` to the cargo invocation for Layer 2.)
# 3. A `customers` table the server can resolve, with columns:
#        id BIGINT, name STRING, region STRING, ssn STRING
#    and a mix of `region` values ('eu' and 'us'). Seed it however your setup
#    resolves tables (e.g. via Unity Catalog + Spark, as in uc_managed.py, or a
#    local registration). The demo only reads it. Set TABLE below to its
#    fully-qualified name.
#
# The demo policy (config/policies/demo.cedar):
#   - Layer 1: permit read when the principal has a `region` attribute.
#   - Layer 2: row filter `region == principal.region` (per-principal); column
#              mask on `ssn` (per-table — see note below).
#
# Expected outcome:
#   alice (region=eu): allowed; sees only eu rows; ssn masked as "***".
#   bob   (region=us): allowed; sees only us rows; ssn masked as "***".
#   carol (no region): DENIED at the gate (query rejected).
#
# Note: the per-principal axis is the ROW FILTER (alice sees eu rows, bob sees us
# rows — the same SQL, different result sets). The column mask is per-table here:
# `ssn` is masked for every caller. Masking keyed on the *concrete principal*
# isn't expressible with the current residual mechanism (a fully-resolved
# principal condition leaves no residual to drive a mask); the mask condition is
# written over the unknown `resource`, so it applies to all callers.

import marimo

__generated_with = "0.18.4"
app = marimo.App()


@app.cell
def _():
    import marimo as mo

    # The Flight SQL endpoint hydrofoil listens on.
    ENDPOINT = "grpc://0.0.0.0:50051"

    # Fully-qualified name of the demo table (see prerequisites). Adjust to match
    # however your environment resolves tables.
    TABLE = "customers"

    QUERY = f"SELECT id, name, region, ssn FROM {TABLE} ORDER BY id"
    return ENDPOINT, QUERY, TABLE, mo


@app.cell
def _(mo):
    mo.md(
        """
        # Cedar policy enforcement demo

        The same query is run as three principals. Only the request **metadata
        headers** change — `x-hydrofoil-principal` and `x-hydrofoil-region`. The
        server resolves the principal from those headers and enforces Cedar
        policy at plan time.
        """
    )
    return


@app.cell
def _(ENDPOINT):
    from adbc_driver_flightsql import DatabaseOptions
    from adbc_driver_flightsql.dbapi import connect

    def run_as(principal: str, query: str, region: str | None = None):
        """Execute `query` as `principal`, sending the hydrofoil principal/region
        metadata headers. Returns (arrow_table, error_string)."""
        headers = {"x-hydrofoil-principal": principal}
        if region is not None:
            headers["x-hydrofoil-region"] = region

        db_kwargs = {
            DatabaseOptions.TLS_SKIP_VERIFY.value: "true",
            **{
                f"{DatabaseOptions.RPC_CALL_HEADER_PREFIX.value}{k}": v
                for k, v in headers.items()
            },
        }
        try:
            with connect(ENDPOINT, db_kwargs=db_kwargs) as conn:
                cur = conn.cursor()
                cur.execute(query)
                table = cur.fetch_arrow_table()
                cur.close()
                return table, None
        except Exception as e:  # noqa: BLE001 — surface the server's deny/error
            return None, str(e)

    return (run_as,)


@app.cell
def _(mo):
    mo.md(
        """
        ## Layer 1 — the access gate

        `carol` carries no `region` attribute, so the demo policy's read
        permission never fires and Cedar's default-deny rejects the **whole
        query** before it executes.
        """
    )
    return


@app.cell
def _(QUERY, mo, run_as):
    _table, _err = run_as('User::"carol"', QUERY, region=None)
    mo.md(
        f"**carol (no region) → expected DENY**\n\n```\n{_err}\n```"
        if _err is not None
        else f"Unexpected: carol was allowed and saw {_table.num_rows} rows."
    )
    return


@app.cell
def _(QUERY, mo, run_as):
    # alice carries region=eu, so the gate permits her.
    _table, _err = run_as('User::"alice"', QUERY, region="eu")
    mo.md(
        f"**alice (region=eu) → expected ALLOW**\n\n```\n{_err}\n```"
        if _err is not None
        else f"alice allowed; sees {_table.num_rows} rows (governed below)."
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        ## Layer 2 — row filter + column mask

        Both principals are allowed by the gate, but governance shapes what they
        see. The **row filter** keeps only rows where `region == principal.region`
        — so alice (eu) and bob (us) get different result sets from the identical
        SQL. The **column mask** hides `ssn` for every caller of this table.

        *(Requires the server built with `--features governance`; without it both
        principals see the full, ungoverned table.)*
        """
    )
    return


@app.cell
def _(QUERY, run_as):
    # alice: region=eu -> eu rows only; ssn masked (per-table mask).
    alice_table, alice_err = run_as('User::"alice"', QUERY, region="eu")
    alice_table if alice_err is None else alice_err
    return alice_err, alice_table


@app.cell
def _(QUERY, run_as):
    # bob: region=us -> us rows only; ssn masked (per-table mask).
    bob_table, bob_err = run_as('User::"bob"', QUERY, region="us")
    bob_table if bob_err is None else bob_err
    return bob_err, bob_table


@app.cell
def _(alice_table, bob_table, mo):
    # Side-by-side summary of what each principal saw.
    def _summary(name, table):
        if table is None:
            return f"- **{name}**: denied / error"
        regions = sorted(set(table.column("region").to_pylist())) if table.num_rows else []
        ssns = set(table.column("ssn").to_pylist()) if table.num_rows else set()
        masked = ssns == {"***"} and table.num_rows > 0
        return (
            f"- **{name}**: {table.num_rows} rows, regions={regions}, "
            f"ssn={'masked (***)' if masked else 'visible'}"
        )

    mo.md(
        "### What each principal saw\n\n"
        + _summary("alice (eu)", alice_table)
        + "\n"
        + _summary("bob (us)", bob_table)
        + "\n\n"
        "alice and bob ran the identical SQL: the **row sets differ by region** "
        "(the per-principal row filter), while `ssn` is **masked for both** (the "
        "per-table column mask) — all from Cedar governance applied at plan time."
    )
    return


if __name__ == "__main__":
    app.run()
