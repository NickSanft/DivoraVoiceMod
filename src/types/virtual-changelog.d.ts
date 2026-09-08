// Ambient type for the build-time virtual module served by
// vite/changelog-plugin.ts. See src/data/changelog-parse.ts for the shape.
declare module "virtual:changelog" {
  import type { ReleaseNote } from "../data/changelog-parse";
  const notes: ReleaseNote[];
  export default notes;
}
