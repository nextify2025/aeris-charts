import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const dist = join(root, "dist");
const snapshot_path = join(root, "api", "public-api-v1.json");
assert.ok(existsSync(dist), "dist is missing; run bun run build first");

const walk = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
  const path = join(dir, entry.name);
  return entry.isDirectory() ? walk(path) : [path];
});
const public_declarations = new Set([
  "accessibility.d.ts", "builtin_plugins.d.ts", "canvas_plugins.d.ts", "custom_series.d.ts",
  "errors.d.ts", "feature_series.d.ts", "grid.d.ts", "index.d.ts", "offscreen.d.ts",
  "primitive_features.d.ts", "primitives.d.ts", "react.d.ts", "shortcuts.d.ts", "theme.d.ts", "types.d.ts",
]);
const files = walk(dist)
  .filter((path) => path.endsWith(".d.ts"))
  .filter((path) => public_declarations.has(relative(dist, path).replaceAll("\\", "/")))
  .sort()
  .map((path) => {
    const text = readFileSync(path, "utf8").replaceAll("\r\n", "\n");
    return {
      path: relative(dist, path).replaceAll("\\", "/"),
      bytes: Buffer.byteLength(text),
      sha256: createHash("sha256").update(text).digest("hex"),
    };
  });
const snapshot = `${JSON.stringify({ schema: 1, files }, null, 2)}\n`;

if (process.argv.includes("--update")) {
  writeFileSync(snapshot_path, snapshot);
  console.log(`updated ${snapshot_path}`);
} else {
  assert.equal(readFileSync(snapshot_path, "utf8"), snapshot,
    "public TypeScript declarations changed; review them, then run bun run update:api");
  console.log(`public API snapshot OK (${files.length} declaration files)`);
}
