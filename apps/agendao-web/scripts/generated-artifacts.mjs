import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const check = process.argv.includes("--check");

const tasks = [
  {
    name: "context-pressure",
    script: resolve(__dirname, "generate-context-pressure.mjs"),
  },
  {
    name: "context-closure",
    script: resolve(__dirname, "generate-context-closure.mjs"),
  },
];

for (const task of tasks) {
  const result = spawnSync(process.execPath, [task.script, ...(check ? ["--check"] : [])], {
    cwd: __dirname,
    stdio: "inherit",
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

if (!check) {
  console.log(
    `Generated ${tasks.length} web artifact${tasks.length === 1 ? "" : "s"}: ${tasks.map((task) => task.name).join(", ")}.`,
  );
}
