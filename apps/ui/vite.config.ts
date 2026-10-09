import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// One app for both targets: the Tauri shell loads `dist/`, the web build serves it.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "es2022", outDir: "dist" },
});
