import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  test: {
    environment: "jsdom",
    globals: true,
    // Unit tests live in src/. The e2e/ specs are Playwright (own runner),
    // so scope Vitest to src/ to avoid picking up *.spec.ts under e2e/.
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    setupFiles: ["./src/test-setup.ts"],
    coverage: {
      provider: "v8",
      reporter: ["text", "html"],
    },
  },
});
