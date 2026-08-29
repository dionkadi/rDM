import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed dev-server port and no cleared screen.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: false,
  },
  build: {
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
  envPrefix: ["VITE_", "TAURI_"],
});
