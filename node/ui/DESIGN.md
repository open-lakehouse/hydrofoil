# Design — `@open-lakehouse/ui` (hydrofoil's unified UI)

hydrofoil's web/desktop shell is the **unifying consumer**: it composes the
shared, headless UI packages into one experience. It owns the concrete look;
the packages own behavior and stay theme-agnostic.

Follows the [`design.md`](https://github.com/google-labs-code/design.md)
convention — this file captures app-level design intent; the cross-repo *token
contract* is defined once in the shared kit and referenced here.

## Where the design lives

- **Token contract (shared):** `@open-lakehouse/ui-kit`'s `DESIGN.md` (in the
  sibling mangrove repo) defines the semantic CSS-variable names every consumer
  must supply (`--background`, `--primary`, `--sidebar*`, `--status-*`, …). All
  three UIs (this app, headwaters' lineage app, mangrove's UC app) align on those
  names so they stay visually coherent while independently themeable.
- **Token values (this app):** `src/app/globals.css`. This is the canonical
  reference implementation of the contract — the light/dark palettes, the
  Tailwind `@theme inline` mapping, and the `@source` globs that scan the linked
  package sources so their utilities are generated. Other apps may choose
  different values; only the variable names are the contract.

## App look & feel

- **IDE-like density:** a VS Code-style shell — sidebar (`--sidebar*`) slightly
  off the editor surface, thin theme-aware scrollbars, `Inter` / `JetBrains Mono`.
- **Semantic-first color:** components reference semantic utilities
  (`bg-background`, `text-muted-foreground`, `border-border`), never raw palette
  values, so a token change re-themes the whole surface.
- **Light/dark:** toggled via the `.dark` class on `<html>` (ui-kit's
  `ThemeProvider`); every token has both a `:root` and `.dark` value.

## Headless boundary (how the shared packages plug in)

The packages carry no theme and no host wiring; this app injects both:

- **Tokens:** supplied by `globals.css` (above) + the `@source` globs.
- **Transport:** `setDefaultUnityCatalogFetch(clientFetch)` in `main.tsx` routes
  UC calls through the host fetch (web fetch, or the Tauri IPC transport on
  desktop). ConnectRPC uses the app's transport registry.
- **Environment scope:** `ActiveEnvironmentProvider` mounts the UC package's
  `EnvironmentScopeProvider` with the active environment id.

Because the packages live in the sibling mangrove repo (consumed via `file:`
links during this evaluation phase), Vite `resolve.dedupe` + tsconfig `paths`
pin React and the TanStack singletons to a single copy across the two installs —
see `vite.config.ts` / `tsconfig.json`.
