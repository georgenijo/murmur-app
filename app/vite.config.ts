import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { execFileSync } from "node:child_process";
import { resolve } from "path";

function uiLatencyBuildId(): string {
  const explicit = process.env.MURMUR_BUILD_ID?.trim();
  if (explicit) return explicit;
  try {
    const revision = execFileSync("git", ["rev-parse", "--short=8", "HEAD"], {
      cwd: __dirname,
      encoding: "utf8",
    }).trim();
    const dirty = execFileSync("git", ["status", "--porcelain"], {
      cwd: __dirname,
      encoding: "utf8",
    }).trim().length > 0;
    return `${revision}${dirty ? "-dirty" : ""}`;
  } catch {
    return "unknown-revision";
  }
}

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],
  define: {
    "import.meta.env.VITE_MURMUR_BUILD_ID": JSON.stringify(uiLatencyBuildId()),
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // 3. tell vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  // Multi-page production windows. Vite serves visual-fixtures.html in dev for
  // Playwright, but it is deliberately excluded from packaged builds.
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        diagnostics: resolve(__dirname, "diagnostics.html"),
        overlay: resolve(__dirname, "overlay.html"),
        "transform-review": resolve(__dirname, "transform-review.html"),
        "query-review": resolve(__dirname, "query-review.html"),
        "dictation-preview": resolve(__dirname, "dictation-preview.html"),
      },
    },
  },
}));
