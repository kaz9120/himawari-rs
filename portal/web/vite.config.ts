import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/** serverの既定ポート（server/src/config.ts の DEFAULT_PORT） */
const SERVER_ORIGIN = "http://localhost:8790";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5174,
    proxy: {
      "/api": { target: SERVER_ORIGIN, changeOrigin: true },
      "/ws": { target: SERVER_ORIGIN, ws: true },
    },
  },
  build: { outDir: "dist", emptyOutDir: true },
});
