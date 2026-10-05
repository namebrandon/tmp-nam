import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Port 1431 so this app's dev server coexists with TMP Companion (1421).
const port = 1431;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "safari15",
    minify: !process.env.TAURI_DEBUG,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
