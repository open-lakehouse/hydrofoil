import {
  createLazyRoute,
  useNavigate,
  useSearch,
} from "@tanstack/react-router";
import { useCallback, useMemo, useState } from "react";
import {
  EditorSessionProvider,
  useEditorSession,
} from "@/components/editor/EditorSessionContext";
import { FileTree } from "@/components/editor/fileTree/FileTree";
import { FileExpansionProvider } from "@/components/editor/fileTree/fileExpansion";
import { MarkdownPreview } from "@/components/editor/MarkdownPreview";
import { MonacoHost } from "@/components/editor/MonacoHost";
import { NotebookPane } from "@/components/editor/NotebookPane";
import { ResultsPane } from "@/components/editor/ResultsPane";
import { TabStrip } from "@/components/editor/TabStrip";
import { useTabPersistence } from "@/components/editor/useTabPersistence";
import { VolumeSwitcher } from "@/components/editor/VolumeSwitcher";
import { useActiveEnvironment } from "@/components/environment/ActiveEnvironmentContext";
import {
  loadAddedVolumes,
  persistAddedVolumes,
  type Volume,
} from "@/lib/editor/volumes";

export const Route = createLazyRoute("/editor")({
  component: EditorPage,
});

const FROM = "/editor";

function EditorPage() {
  return (
    <EditorSessionProvider>
      <FileExpansionProvider>
        <Workspace />
      </FileExpansionProvider>
    </EditorSessionProvider>
  );
}

function Workspace() {
  const { tabs, activeId, openFile, notebookController } = useEditorSession();
  useTabPersistence();
  const activeTab = tabs.find((t) => t.id === activeId);
  const isSql = activeTab?.language === "sql";
  const isMarkdown = activeTab?.language === "markdown";
  const isNotebook = activeTab?.language === "notebook";
  // The notebook controller for the active tab (null for non-notebook tabs or
  // when no host is registered). Resolving it here keeps NotebookPane keyed to
  // the active path so its iframe persists across tab switches.
  const nbController =
    isNotebook && activeId ? notebookController(activeId) : null;

  // The set of selectable volumes: the active environment's built-in volumes
  // (the local Home on desktop) plus any UC volumes the user has browsed to
  // (persisted, namespaced per environment).
  const env = useActiveEnvironment();
  const builtin = env.volumes;
  const [added, setAdded] = useState<Volume[]>(() => loadAddedVolumes(env.id));
  const volumes = useMemo(() => [...builtin, ...added], [builtin, added]);

  // The active volume's root lives in the URL (`?volume=`), defaulting to the
  // first available volume. The FileTree re-roots whenever this changes.
  const navigate = useNavigate({ from: FROM });
  const urlVolume = useSearch({ from: FROM, select: (s) => s.volume });
  const activeRoot = urlVolume ?? volumes[0]?.root;

  const selectVolume = useCallback(
    (root: string) => {
      navigate({
        search: (prev) => ({ ...prev, volume: root }),
        replace: true,
      });
    },
    [navigate],
  );

  const addVolume = useCallback(
    (volume: Volume) => {
      setAdded((prev) => {
        const next = prev.some((v) => v.id === volume.id)
          ? prev
          : [...prev, volume];
        persistAddedVolumes(env.id, next);
        return next;
      });
      selectVolume(volume.root);
    },
    [selectVolume, env.id],
  );

  return (
    <div className="flex h-full">
      <div className="flex w-64 shrink-0 flex-col border-r">
        <VolumeSwitcher
          volumes={volumes}
          activeRoot={activeRoot}
          onSelect={selectVolume}
          onAdd={addVolume}
        />
        {activeRoot ? (
          <FileTree
            // Re-key on the root so the tree's expansion/local state resets when
            // switching volumes (the cache is keyed per-path regardless).
            key={activeRoot}
            root={activeRoot}
            activePath={activeId ?? undefined}
            onOpenFile={(path) => void openFile(path)}
          />
        ) : (
          <div className="p-4 text-xs text-muted-foreground">
            No volume selected. Add a volume to start browsing files.
          </div>
        )}
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <TabStrip />
        {/* The single MonacoHost is shared across layouts. SQL tabs stack the
            editor over a results pane; markdown tabs place a preview beside the
            editor; notebook tabs replace the editor area with an embedded marimo
            iframe; other tabs are editor-only. The Monaco editor area is hidden
            (not unmounted) while a notebook tab is active, so its models and
            view state survive switching back. */}
        <div className="flex min-h-0 flex-1">
          {isNotebook && nbController && activeId ? (
            <NotebookPane key={activeId} controller={nbController} />
          ) : (
            <>
              <div className="flex min-w-0 flex-1 flex-col">
                <div className="min-h-0 flex-1">
                  <MonacoHost />
                </div>
                {isSql && activeId && (
                  <div className="h-2/5 min-h-0 border-t">
                    <ResultsPane activePath={activeId} />
                  </div>
                )}
              </div>
              {isMarkdown && activeId && (
                <div className="min-h-0 w-1/2 shrink-0">
                  <MarkdownPreview activePath={activeId} />
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
