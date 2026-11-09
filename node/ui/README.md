# hydrofoil UI

The hydrofoil web UI: query, ingest, files and tags (`just node ui-dev`,
`just node ui-storybook`). The desktop app that used to embed it was removed;
the desktop shell for local lakehouse environments is luminescent-lakehouse's
[`apps/desktop`](https://github.com/open-lakehouse/luminescent-lakehouse/tree/main/apps/desktop).

## Known gap: sibling links

`package.json` takes four packages from a mangrove checkout next to this repo
(`file:../../../mangrove/node/{data-grid,ui-kit,unity-catalog,unity-catalog-client}`).
mangrove has since moved or renamed some of them, and trestle publishes the
canonical `@open-lakehouse/ui-kit`, so `npm install` only works against a
mangrove checkout from that time. Moving these to published versions is part
of the separate hydrofoil UI track (server image, trestle module, published UI
package). It isn't a drop-in swap: the published `ui-kit` (0.0.x) isn't the
API of the `^0.1.0` fork these imports were written against.
