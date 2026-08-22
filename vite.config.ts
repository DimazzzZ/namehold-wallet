import { defineConfig } from "vite";
import { resolve } from "path";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import pkg from "./package.json" with { type: "json" };

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  // Inject the version from package.json at build time so consumers (webqa
  // mock, any future frontend-only surface) don't have to hardcode it. The
  // real Tauri app already reads the version from the backend via the
  // `current_version` command; this is the fallback for non-Tauri builds.
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  build: {
    target: ["es2021", "chrome100", "safari13"],
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
    rollupOptions: {
      input: {
        // Main React app.
        main: resolve(__dirname, "index.html"),
        // Standalone Rust-owned secure prompt surface (no React).
        secure: resolve(__dirname, "secure.html"),
      },
      output: {
        // Split heavy vendor libraries into their own chunks so no single
        // chunk crosses Vite's 500 kB warning threshold. Desktop app: no
        // network latency cost — this is purely for build hygiene and
        // dev-mode caching.
        manualChunks(id) {
          if (id.includes("node_modules")) {
            if (id.includes("react-dom")) return "vendor-react";
            if (
              id.includes("react-markdown") ||
              id.includes("remark") ||
              id.includes("rehype") ||
              id.includes("unified") ||
              id.includes("micromark") ||
              id.includes("mdast") ||
              id.includes("hast")
            )
              return "vendor-markdown";
            if (id.includes("@tanstack")) return "vendor-tanstack";
            return "vendor";
          }
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["src/test/setup.ts"],
  },
});
