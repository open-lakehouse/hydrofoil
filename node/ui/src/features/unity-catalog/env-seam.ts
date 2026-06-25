// The single seam between the Unity Catalog module and the host's environment
// state.
//
// `ExpansionContext` namespaces its persisted tree-expansion state per active
// environment, so it needs the current environment id. That is the ONE outbound
// dependency from UC core that is not itself UC — every other dependency is a
// shared primitive (@/components/ui/*, @/components/forms/*, @/lib/utils) or the
// generic client factory (@/lib/api).
//
// Concentrating that edge here keeps it the single documented thing an
// extraction would need to parameterize: lift the module out and replace this
// file's import with whatever supplies the embedder's "current scope" id (or a
// constant), and nothing in core changes. See ./README.md.
import { useActiveEnvironment } from "@/components/environment/ActiveEnvironmentContext";

/**
 * The id used to namespace per-environment UI state (currently tree expansion).
 * Backed by the host's active-environment context today.
 */
export function useEnvironmentScopeId(): string {
  return useActiveEnvironment().id;
}
