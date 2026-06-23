// Idempotent DOM tweaks for an embedded marimo document. Shared by the
// gateway-backed service surface (lib/services.ts) and the desktop notebook tab
// (components/editor/NotebookPane.tsx). Only safe to call when the iframe is
// same-origin (it is for both: the service prefix and the `olservice://`
// notebook protocol are same-origin to the embedding document). Called once on
// load and again on DOM mutations, so it must tolerate repeated runs.
//
// marimo's workspace/home page renders a "Resources" block and a "Tutorials"
// dropdown that are hard-coded in its frontend with no server-config toggle
// (and `display.custom_css` is a no-op on the home page). We strip them
// client-side from the embedded document instead.
export function customizeMarimoFrame(doc: Document): void {
  // Stable hooks: hide via a single injected stylesheet (idempotent by id).
  const STYLE_ID = "ol-marimo-customizations";
  if (!doc.getElementById(STYLE_ID)) {
    const style = doc.createElement("style");
    style.id = STYLE_ID;
    style.textContent = `
      /* Tutorials dropdown trigger on the workspace home page. */
      [data-testid="open-tutorial-button"] { display: none !important; }
    `;
    doc.head.append(style);
  }

  // The "Resources" block has no stable selector, so anchor on its heading
  // text. Structure (marimo home/components.tsx): a section <div> whose first
  // child is the Header's <div>, which contains an <h2> with the text
  // "Resources". Two levels up from that <h2> is the section root.
  for (const heading of doc.querySelectorAll("h2")) {
    if (heading.textContent?.trim() === "Resources") {
      const section = heading.parentElement?.parentElement;
      if (section instanceof HTMLElement) {
        section.style.display = "none";
      }
    }
  }
}
