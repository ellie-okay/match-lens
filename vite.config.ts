import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";

const env = (
  globalThis as typeof globalThis & {
    process?: { env?: Record<string, string | undefined> };
  }
).process?.env ?? {};

const host = env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [sveltekit()],

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: !env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!env.TAURI_ENV_DEBUG,
  },
});
