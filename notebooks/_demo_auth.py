"""Shared demo-auth helper for the marimo notebooks.

Lets a notebook run queries against hydrofoil *as a chosen user*, forwarding both
the Cedar principal (`x-hydrofoil-principal`) and that user's Unity Catalog bearer
token (`x-hydrofoil-uc-token`) so UC enforces the user's own permissions. The
principal UID encodes the user's email — `User::"alice@example.com"` — which is
the identity hydrofoil and UC agree on.

**Tokens are never rendered.** Users are listed by email; each email maps to an
*environment-variable name* (e.g. `alice@example.com` -> `UC_TOKEN_ALICE`), and
the token value is resolved from `os.environ` only at the moment a connection is
built. The dropdown options and notebook output only ever show emails, never the
token. Supply the tokens via a gitignored `notebooks/.env` (auto-loaded by marimo;
see `notebooks/pyproject.toml`) — mint them with `just mint-demo-tokens`.

A missing token degrades gracefully: the header is simply omitted and hydrofoil
falls back to its server-wide UC token (no per-user enforcement), rather than
erroring.

This module is *imported* by the notebooks (it carries no PEP 723 block); it has
no third-party dependencies, so it works in `--sandbox` runs too.
"""

from __future__ import annotations

import os

# ADBC Flight SQL option keys. Hardcoded (rather than importing the
# `DatabaseOptions` enum) so the same strings work in DuckDB's `adbc_connect`
# flat-map path, which takes plain string keys.
RPC_CALL_HEADER_PREFIX = "adbc.flight.sql.rpc.call_header."
TLS_SKIP_VERIFY = "adbc.flight.sql.client_option.tls_skip_verify"

# Header keys hydrofoil reads (see crates/hydrofoil/src/identity.rs). The UC token
# rides `x-hydrofoil-uc-token` — a *distinct* key from gRPC `authorization`, which
# the Flight path uses as its session-id channel.
PRINCIPAL_HEADER = "x-hydrofoil-principal"
UC_TOKEN_HEADER = "x-hydrofoil-uc-token"
REGION_HEADER = "x-hydrofoil-region"

# Default demo roster. Override with UC_DEMO_USERS (comma-separated emails).
_DEFAULT_USERS = ["alice@example.com", "bob@example.com"]


def _env_var_name(email: str) -> str:
    """Map an email to its token env-var name: alice@x.com -> UC_TOKEN_ALICE."""
    local = email.split("@", 1)[0]
    slug = "".join(ch if ch.isalnum() else "_" for ch in local).upper()
    return f"UC_TOKEN_{slug}"


def _load_users() -> dict[str, str]:
    """Ordered {email: env-var-name} from UC_DEMO_USERS, else the default roster."""
    raw = os.environ.get("UC_DEMO_USERS", "")
    emails = [e.strip() for e in raw.split(",") if e.strip()] or _DEFAULT_USERS
    return {email: _env_var_name(email) for email in emails}


#: Ordered mapping of demo user email -> token env-var name. The env-var *name*
#: is safe to display; the value (the token) is not and is read lazily.
USERS: dict[str, str] = _load_users()


def principal_for(email: str) -> str:
    """The Cedar principal UID for a user — the email-encoded entity UID."""
    return f'User::"{email}"'


def token_for(email: str) -> str | None:
    """The user's UC token from the environment, or None when unset/empty.

    None means "no per-user token" — callers omit the header and hydrofoil uses
    its shared server-wide token.
    """
    name = USERS.get(email) or _env_var_name(email)
    token = os.environ.get(name)
    return token or None


def call_headers(
    email: str,
    *,
    region: str | None = None,
    extra: dict[str, str] | None = None,
) -> dict[str, str]:
    """Build the hydrofoil RPC call headers for `email`.

    Always sets `x-hydrofoil-principal`; adds `x-hydrofoil-uc-token` when a token
    is available, optional `x-hydrofoil-region`, and merges any `extra` headers
    (e.g. OpenLineage metadata). The token value is placed under the UC-token
    header key only — never logged or surfaced under a display key.
    """
    headers = {PRINCIPAL_HEADER: principal_for(email)}
    token = token_for(email)
    if token:
        headers[UC_TOKEN_HEADER] = token
    if region is not None:
        headers[REGION_HEADER] = region
    if extra:
        headers.update(extra)
    return headers


def db_kwargs(
    email: str,
    *,
    region: str | None = None,
    extra: dict[str, str] | None = None,
    tls_skip_verify: bool = True,
) -> dict[str, str]:
    """ADBC `db_kwargs` for `connect(ENDPOINT, db_kwargs=...)` as `email`.

    Prefixes each call header with `RPC_CALL_HEADER_PREFIX` and (by default) skips
    TLS verification, matching the existing notebook connections.
    """
    kwargs: dict[str, str] = {}
    if tls_skip_verify:
        kwargs[TLS_SKIP_VERIFY] = "true"
    for key, value in call_headers(email, region=region, extra=extra).items():
        kwargs[f"{RPC_CALL_HEADER_PREFIX}{key}"] = value
    return kwargs


def user_dropdown(mo, *, label: str = "Demo user", value: str | None = None):
    """A `mo.ui.dropdown` of demo user emails. `.value` is the selected email.

    Options are emails only — never tokens — so nothing sensitive is rendered.
    """
    emails = list(USERS.keys())
    return mo.ui.dropdown(
        options=emails,
        value=value or (emails[0] if emails else None),
        label=label,
    )
