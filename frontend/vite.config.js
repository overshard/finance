import { resolve } from "path";
import { defineConfig } from "vite";

// Mirrors the sibling Rust apps:
// - Entry points live under static_src/{base,home,symbol,health,search}
// - Vite outputs to ../dist/ with content-hashed filenames
// - The Rust binary reads dist/.vite/manifest.json to resolve hashed names
export default defineConfig({
  base: "/static/",
  build: {
    outDir: resolve(__dirname, "../dist"),
    emptyOutDir: true,
    manifest: true,
    rollupOptions: {
      input: {
        base: resolve(__dirname, "static_src/base/index.js"),
        home: resolve(__dirname, "static_src/home/index.js"),
        symbol: resolve(__dirname, "static_src/symbol/index.js"),
        health: resolve(__dirname, "static_src/health/index.js"),
        search: resolve(__dirname, "static_src/search/index.js"),
        backtest: resolve(__dirname, "static_src/backtest/index.js"),
        industries: resolve(__dirname, "static_src/industries/index.js"),
      },
    },
  },
  css: {
    preprocessorOptions: {
      scss: { quietDeps: true },
    },
  },
});
