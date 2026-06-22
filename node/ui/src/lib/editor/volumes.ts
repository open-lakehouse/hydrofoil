// The editor's notion of an "active volume": the file tree is rooted at one
// volume's file-API path at a time. A volume is either the always-available local
// "home" (desktop only) or a Unity Catalog volume addressed by its Databricks
// Volumes path.

export interface Volume {
  /** Stable id (the root path doubles as the id). */
  id: string;
  /** Display label, e.g. "Home" or "main.default.data". */
  label: string;
  /** The file-API root path the tree mounts at. */
  root: string;
}

/** The local home volume (served by the desktop host at `/home`). */
export const HOME_VOLUME: Volume = {
  id: "/home",
  label: "Home",
  root: "/home",
};

/** Build the Databricks Volumes path for a UC volume. The file APIs (and the
 *  Rust `UnityVolumeStore`) address volumes as `/Volumes/<catalog>/<schema>/<volume>`. */
export function volumePath(parts: {
  catalog: string;
  schema: string;
  volume: string;
}): string {
  return `/Volumes/${parts.catalog}/${parts.schema}/${parts.volume}`;
}

/** A UC volume as a `Volume`, labeled by its dot-separated full name. */
export function ucVolume(parts: {
  catalog: string;
  schema: string;
  volume: string;
}): Volume {
  const root = volumePath(parts);
  return {
    id: root,
    label: `${parts.catalog}.${parts.schema}.${parts.volume}`,
    root,
  };
}

// Added UC volumes persist across reloads (like the open-tab set), so the
// switcher remembers what the user has browsed to.
const STORAGE_KEY = "editor.volumes.added";

export function loadAddedVolumes(): Volume[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as Volume[];
  } catch {
    // ignore malformed storage
  }
  return [];
}

export function persistAddedVolumes(volumes: Volume[]): void {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(volumes));
  } catch {
    // storage may be unavailable (private mode etc.)
  }
}
