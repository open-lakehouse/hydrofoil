# /// script
# requires-python = ">=3.13"
# dependencies = [
#     "duckdb>=1.5.0",
#     "adbc-driver-flightsql>=1.9.0",
#     "marimo",
#     "pyarrow==24.0.0",
# ]
# ///

# Reach hydrofoil's Arrow Flight SQL endpoint *through DuckDB*, from a marimo SQL cell.
#
# marimo SQL cells run on DuckDB. To let DuckDB talk to a remote Arrow Flight SQL server we
# use the `adbc_scanner` community extension, which loads a native ADBC driver and scans its
# results as DuckDB tables. We point it at the *same* Flight SQL ADBC driver that the Python
# notebooks (client.py / policy_demo.py) already use against hydrofoil — so the protocol
# matches: handshake -> CommandStatementQuery -> ticketed result.
#
# Why `adbc_scanner` and not the `airport` extension: `airport` is a *generic* Arrow Flight
# client with its own metadata conventions; it does NOT speak the Flight SQL command protocol,
# and a `SELECT 1` under airport would execute locally in DuckDB — proving nothing about the
# server. With `adbc_scanner`, the `SELECT 1` string is sent *to hydrofoil* and the result
# comes back, which is exactly the connectivity test we want.
#
# We keep the test focused on connectivity: `SELECT 1` reads no data. Hydrofoil routes every
# query through its Cedar gate, but the default policy is allow-all and `SELECT 1` is a
# constant projection (no DDL/DML), so it passes cleanly.
#
# ── Prerequisites ───────────────────────────────────────────────────────────
#   1. Hydrofoil running and reachable at grpc://localhost:50051. On the host:
#          just env-up      # brings up the compose stack (unity-catalog, etc.)
#          just hydro       # runs hydrofoil on the host at :50051 (RUST_LOG=hydrofoil=debug)
#   2. Run this notebook on the host (the marimo container has no outbound network, so it
#      can't INSTALL the community extension):
#          uvx --directory notebooks/ marimo edit --sandbox duckdb_flight.py
#
# If `adbc_scan` misbehaves, run client.py first — it does the same `SELECT 1` directly via
# the Python ADBC driver, isolating any failure to the extension layer rather than hydrofoil.

import marimo

__generated_with = "0.23.8"
app = marimo.App(width="medium")


@app.cell
def _():
    import marimo as mo

    # The Flight SQL endpoint hydrofoil listens on (host port-mapped from the container).
    ENDPOINT = "grpc://hydrofoil:50051"
    return ENDPOINT, mo


@app.cell(hide_code=True)
def _(mo):
    mo.md("""
    # DuckDB → hydrofoil over Flight SQL

    A marimo **SQL cell** (DuckDB) reaches the **hydrofoil** Arrow Flight SQL service via
    the `adbc_scanner` extension. The query `SELECT 1` is sent *to hydrofoil* and the
    single-row result comes back into DuckDB — a focused connectivity check that reads no
    data.
    """)
    return


@app.cell(hide_code=True)
def _():
    # adbc_scanner loads a native ADBC driver shared library. The Flight SQL driver ships
    # bundled inside the adbc-driver-flightsql wheel; ask the package for its path rather than
    # hardcoding it (the file is libadbc_driver_flightsql.dylib on macOS, .so on Linux).
    import adbc_driver_flightsql

    if hasattr(adbc_driver_flightsql, "_driver_path"):
        driver_path = adbc_driver_flightsql._driver_path()
    else:
        # Fallback for driver versions without the helper: glob the bundled library.
        import glob
        import os

        pkg_dir = os.path.dirname(adbc_driver_flightsql.__file__)
        matches = glob.glob(os.path.join(pkg_dir, "libadbc_driver_flightsql.*"))
        driver_path = matches[0]
    driver_path
    return (driver_path,)


@app.cell
def _(mo):
    # Pick the UC user to connect as; their token (if configured in notebooks/.env)
    # is forwarded so UC enforces their permissions. Only the email is shown.
    import _demo_auth

    user = _demo_auth.user_dropdown(mo)
    user
    return _demo_auth, user


@app.cell
def _(ENDPOINT, _demo_auth, driver_path, user):
    # Install/load adbc_scanner, then open an ADBC Flight SQL connection to hydrofoil. The
    # connection handle is a BIGINT we stash in a DuckDB variable for the scan cell.
    #
    # The x-hydrofoil-principal / x-hydrofoil-uc-token RPC call headers carry the selected
    # user's identity and UC token, so hydrofoil resolves a real principal and (when a token
    # is configured) UC enforces that user's permissions. Whether adbc_scanner forwards
    # unrecognized option keys to the driver is undocumented, so don't treat the headers
    # arriving as load-bearing for a bare SELECT.
    import duckdb

    con = duckdb.connect()
    con.execute("INSTALL adbc_scanner FROM community; LOAD adbc_scanner;")
    con.execute("SET VARIABLE flightsql_driver = ?;", [driver_path])

    # Build the adbc_connect option struct: driver + uri + the user's call headers
    # (already prefixed with adbc.flight.sql.rpc.call_header.*). DuckDB struct keys are
    # SQL string literals, so render them; the token value never appears in notebook
    # *output*, only inside this connect call.
    _opts = {"driver": "getvariable('flightsql_driver')", "uri": f"'{ENDPOINT}'"}
    for _k, _v in _demo_auth.db_kwargs(user.value).items():
        if _k.startswith(_demo_auth.RPC_CALL_HEADER_PREFIX):
            _opts[_k] = f"'{_v}'"
    _struct = ", ".join(
        # 'driver'/'uri' values are expressions; header values are quoted literals.
        f"'{_k}': {_v}" for _k, _v in _opts.items()
    )
    con.execute(f"SET VARIABLE conn = (SELECT adbc_connect({{{_struct}}}));")
    return (con,)


@app.cell
def _(con):
    query = "SELECT id, event FROM demo.managed_demo.events WHERE id > 1"
    con.execute(f"SELECT * FROM adbc_scan(getvariable('conn')::BIGINT, '{query}')").to_arrow_table()
    return


@app.cell
def _(con):
    # Close the ADBC connection.
    con.execute("SELECT adbc_disconnect(getvariable('conn')::BIGINT);")
    return


if __name__ == "__main__":
    app.run()
