// v1.49.0: serves the parsed CHANGELOG to the app as `virtual:changelog`.
//
// The parse happens here, at build time, so the bundle carries a small
// typed array rather than a 215 KB markdown file and a runtime parser.
// Registered in BOTH vite.config.ts and vitest.config.ts (they're separate
// configs); importing this one factory from both is what keeps them in step.
//
// Failure is loud on purpose — a CHANGELOG whose headings this can't read
// throws during the build rather than shipping users an empty panel.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import type { Plugin } from "vite";
import { parseChangelog } from "../src/data/changelog-parse";

const VIRTUAL_ID = "virtual:changelog";
const RESOLVED_ID = "\0virtual:changelog";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CHANGELOG_PATH = join(REPO_ROOT, "CHANGELOG.md");

/** How many releases the bundle carries. Must stay >= the render cap in
 *  src/data/changelog.ts, or the "and N earlier releases" count lies. */
export const BUNDLED_RELEASES = 12;

export function changelogPlugin(): Plugin {
  return {
    name: "divora-changelog",
    resolveId(id) {
      return id === VIRTUAL_ID ? RESOLVED_ID : null;
    },
    load(id) {
      if (id !== RESOLVED_ID) return null;
      const md = readFileSync(CHANGELOG_PATH, "utf8");
      const notes = parseChangelog(md, BUNDLED_RELEASES);
      if (notes.length === 0) {
        throw new Error(
          "[changelog] parsed zero releases — the panel would ship empty.",
        );
      }
      return `export default ${JSON.stringify(notes)};`;
    },
    configureServer(server) {
      // Editing CHANGELOG.md during dev should refresh the panel.
      server.watcher.add(CHANGELOG_PATH);
      server.watcher.on("change", (file) => {
        if (file !== CHANGELOG_PATH) return;
        const mod = server.moduleGraph.getModuleById(RESOLVED_ID);
        if (mod) server.moduleGraph.invalidateModule(mod);
        server.ws.send({ type: "full-reload" });
      });
    },
  };
}
