import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { changelogPlugin } from "./vite/changelog-plugin";

export default defineConfig({
  plugins: [solid(), changelogPlugin()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    // v1.16.0: a second entry for the transparent stream-overlay window
    // (overlay.html → src/overlay.tsx), alongside the main app.
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        overlay: fileURLToPath(new URL("./overlay.html", import.meta.url)),
      },
    },
  },
});
