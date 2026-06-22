// Tauri desktop entry point.
//
// This runs BEFORE the UI bootstraps. It installs the Tauri fetch into the UI's
// generic fetch registry, then hands off to the existing UI app — which is reused
// wholesale, with no knowledge that it's running inside Tauri.
//
// `@` resolves to ../ui/src (see vite.config.ts / tsconfig.json), so we import the
// UI's own modules directly without adding any `exports` surface to @open-lakehouse/ui.
import { registerFetch } from "@/lib/client/registry";
// Desktop stylesheet: re-exports the UI's globals AND declares the UI source as a
// Tailwind content root, so utility classes used by the UI components are emitted
// when building from node/desktop (see styles.css).
import "./styles.css";
import { tauriFetch } from "./tauri-fetch";

registerFetch(tauriFetch);

// Dynamically import the UI bootstrap so it (and the api.ts client it pulls in)
// evaluates only AFTER the fetch is registered. The registry is late-binding so
// order wouldn't actually matter, but this keeps the intent obvious.
void import("@/main");
