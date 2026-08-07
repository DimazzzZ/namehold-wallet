import { defineConfig } from "vite";
import { resolve } from "path";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
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
