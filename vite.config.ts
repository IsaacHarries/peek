import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "tailwindcss";
import autoprefixer from "autoprefixer";
import { resolve } from "path";

// Multi-page Tauri frontend: one entry per window (overlay + setup).
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": resolve(__dirname, "./src"),
    },
  },
  css: {
    postcss: {
      plugins: [tailwindcss(), autoprefixer()],
    },
  },
  // Tauri expects a fixed port and quiet output.
  clearScreen: false,
  server: {
    port: 1431,
    strictPort: true,
  },
  build: {
    rollupOptions: {
      input: {
        overlay: resolve(__dirname, "overlay.html"),
        setup: resolve(__dirname, "setup.html"),
      },
    },
  },
});
