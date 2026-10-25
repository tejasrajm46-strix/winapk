import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Tauri serves the dev frontend from a fixed port and shows the same window the
// release build will use, so the port is strict.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  build: { target: "chrome110", sourcemap: false },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
  },
});
