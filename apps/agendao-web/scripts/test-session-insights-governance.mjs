import { build } from "esbuild";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const rootDir = new URL("..", import.meta.url).pathname;
const tempDir = path.join(rootDir, ".tmp-tests");
await fs.mkdir(tempDir, { recursive: true });
const outfile = path.join(
  await fs.mkdtemp(path.join(tempDir, "governance-")),
  "session-insights-governance-test.mjs",
);

try {
  await build({
    entryPoints: [path.join(rootDir, "scripts/test-session-insights-governance-entry.tsx")],
    outfile,
    bundle: true,
    format: "esm",
    platform: "node",
    target: "node20",
    jsx: "automatic",
    tsconfig: path.join(rootDir, "tsconfig.json"),
    absWorkingDir: rootDir,
    alias: {
      "@": path.join(rootDir, "src"),
    },
    external: ["react", "react/jsx-runtime", "react-dom/server", "node:*"],
    logLevel: "silent",
  });

  await import(pathToFileURL(outfile).href);
} finally {
  await fs.rm(path.dirname(outfile), { recursive: true, force: true });
}
