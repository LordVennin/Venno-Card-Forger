import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { defineConfig, type Plugin } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

/**
 * Dev-only endpoints for `pnpm test:parity`. Active only when the parity script
 * sets CARDSMITH_PARITY_DIR: the frontend sees `/__parity/input`, renders the
 * samples in the WASM core and posts the frame hashes back.
 */
function parity(dir: string | undefined): Plugin {
  return {
    name: "cardsmith-parity",
    configureServer(server) {
      if (!dir) return;
      server.middlewares.use("/__parity/input", (_req, res) => {
        res.setHeader("Content-Type", "application/json");
        res.end("{}");
      });
      server.middlewares.use("/__parity/result", (req, res) => {
        const chunks: Buffer[] = [];
        req.on("data", (c: Buffer) => chunks.push(c));
        req.on("end", async () => {
          await writeFile(join(dir, "result.json"), Buffer.concat(chunks));
          res.end("ok");
        });
      });
    },
  };
}

// Tauri expects a fixed port and its own error output.
export default defineConfig({
  plugins: [svelte(), parity(process.env.CARDSMITH_PARITY_DIR)],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
