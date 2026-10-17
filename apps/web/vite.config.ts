/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [tailwindcss()],
  esbuild: {
    jsx: "automatic",
  },
  server: {
    port: 5173,
    host: "0.0.0.0",
    // The app calls /api/v1 on its own origin. The API server is a separate process in development.
    proxy: { "/api": "http://127.0.0.1:8080" },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
  },
});

