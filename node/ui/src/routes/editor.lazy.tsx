import { createLazyRoute } from "@tanstack/react-router";
import { useState } from "react";
import { FileTree } from "@/components/editor/fileTree/FileTree";
import { FileExpansionProvider } from "@/components/editor/fileTree/fileExpansion";
import { MonacoHost } from "@/components/editor/MonacoHost";
import { Button } from "@/components/ui/button";
import { useInvalidateDirectory } from "@/lib/files/queries";
import { connectFileStore } from "@/lib/files/store";

export const Route = createLazyRoute("/editor")({
  component: EditorPage,
});

const ROOT = "/work";

// INTERIM: file tree (left) + Monaco (right). The tabs/results layers replace
// the single-editor right pane in the next steps. A "seed" button populates the
// in-memory store so the tree has something to browse during desktop validation.
function EditorPage() {
  const [activePath, setActivePath] = useState<string | undefined>();
  const invalidate = useInvalidateDirectory();

  async function seed() {
    const enc = (s: string) => new TextEncoder().encode(s);
    // The memory store's unary listing only surfaces directories that were
    // explicitly created (it doesn't synthesize them from deeper file prefixes),
    // so create the dirs before writing into them.
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
    <FileExpansionProvider>
      <div className="flex h-full">
        <div className="flex w-64 shrink-0 flex-col border-r">
          <FileTree
            root={ROOT}
            activePath={activePath}
            onOpenFile={setActivePath}
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
          <div className="border-b px-3 py-2 text-xs text-muted-foreground">
            {activePath ?? "No file selected"}
          </div>
          <div className="min-h-0 flex-1">
            <MonacoHost />
          </div>
        </div>
      </div>
    </FileExpansionProvider>
  );
}
