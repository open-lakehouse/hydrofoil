import { createLazyRoute } from "@tanstack/react-router";
import {
  EditorSessionProvider,
  useEditorSession,
} from "@/components/editor/EditorSessionContext";
import { FileTree } from "@/components/editor/fileTree/FileTree";
import { FileExpansionProvider } from "@/components/editor/fileTree/fileExpansion";
import { MonacoHost } from "@/components/editor/MonacoHost";
import { ResultsPane } from "@/components/editor/ResultsPane";
import { TabStrip } from "@/components/editor/TabStrip";
import { Button } from "@/components/ui/button";
import { useInvalidateDirectory } from "@/lib/files/queries";
import { connectFileStore } from "@/lib/files/store";

export const Route = createLazyRoute("/editor")({
  component: EditorPage,
});

const ROOT = "/work";

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
  const { tabs, activeId, openFile } = useEditorSession();
  const invalidate = useInvalidateDirectory();
  const activeTab = tabs.find((t) => t.id === activeId);
  const isSql = activeTab?.language === "sql";

  async function seed() {
    const enc = (s: string) => new TextEncoder().encode(s);
    // The memory store's unary listing only surfaces explicitly-created dirs.
    await connectFileStore.createDir(ROOT);
    await connectFileStore.createDir(`${ROOT}/queries`);
    await connectFileStore.writeFile(
      `${ROOT}/queries/top_users.sql`,
      enc(
        "SELECT * FROM main.default.users\nORDER BY events DESC\nLIMIT 10;\n",
      ),
      { contentType: "text/plain" },
    );
    await connectFileStore.writeFile(
      `${ROOT}/queries/daily.sql`,
      enc("SELECT date, count(*) FROM events GROUP BY 1;\n"),
      { contentType: "text/plain" },
    );
    await connectFileStore.writeFile(
      `${ROOT}/README.md`,
      enc("# Work\n\nScratch SQL and notes.\n"),
      { contentType: "text/markdown" },
    );
    await Promise.all([invalidate(ROOT), invalidate(`${ROOT}/queries`)]);
  }

  return (
    <div className="flex h-full">
      <div className="flex w-64 shrink-0 flex-col border-r">
        <FileTree
          root={ROOT}
          activePath={activeId ?? undefined}
          onOpenFile={(path) => void openFile(path)}
        />
        <div className="border-t p-2">
          <Button
            variant="outline"
            size="sm"
            className="w-full text-xs"
            onClick={seed}
          >
            Seed sample files
          </Button>
        </div>
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <TabStrip />
        {/* SQL tabs split editor (top) / results (bottom); other tabs are
            editor-only. The single MonacoHost is shared across both layouts. */}
        <div className="min-h-0 flex-1">
          <MonacoHost />
        </div>
        {isSql && activeId && (
          <div className="h-2/5 min-h-0 border-t">
            <ResultsPane activePath={activeId} />
          </div>
        )}
      </div>
    </div>
  );
}
