import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const retired = "ori" + "gin";
const allowed = [
  new RegExp(`bounds\\.${retired}\\.[xy]`, "gi"),
  new RegExp(`b\\.${retired}\\.x`, "gi"),
  new RegExp(`${retired}:\\s*(?=(?:self\\.point|point|wgpu::))`, "gi"),
  new RegExp(`wgpu::${retired}3d`, "gi"),
  new RegExp(`(?:semantic|progress)\\s+${retired}`, "gi"),
  new RegExp(`${retired}\\s+edge`, "gi"),
  new RegExp(`entry/${retired}`, "gi"),
  new RegExp(`whose\\s+${retired}`, "gi"),
  new RegExp(`cross-${retired}`, "gi"),
  new RegExp(`same-${retired}`, "gi"),
  new RegExp(`cross${retired}isolated`, "gi"),
];
const generatedAllowed = [
  new RegExp(`__wbg_set_${retired}_[0-9a-f]+`, "gi"),
  new RegExp(`\\.${retired}\\b`, "gi"),
  new RegExp(`${retired}al error`, "gi"),
  // React DOM's generated SVG property table contains standards-defined attribute names that
  // include the retired token (the font coordinates, and the CSS transform property since
  // React 19); keep those web-platform spellings exempt.
  new RegExp(`(?:horiz|vert)-${retired}-[xy]`, "gi"),
  new RegExp(`transform-${retired}\\b`, "gi"),
  new RegExp(`${retired}:\\s*["']${retired}["']`, "gi"),
];
const generated = ["packages/charts/pkg", "packages/charts/dist", "examples/web_demo/pkg", "examples/web_demo/dist"];
// Compliance text is copied verbatim from third parties and is not an owned namespace surface.
const content_exempt_prefixes = ["third_party_licenses/"];
const content_exempt_names = new Set(["LICENSE"]);
const failures = [];
const retired_namespace = new RegExp(`(?<![A-Za-z0-9_])${retired}(?![A-Za-z0-9_])`, "i");

function inspect(path, label, generated = false) {
  if (retired_namespace.test(path)) failures.push(`${label}: forbidden path`);
  const lines = readFileSync(path).toString("latin1").split(/\r?\n/);
  lines.forEach((line, index) => {
    let remainder = line;
    for (const pattern of allowed) remainder = remainder.replace(pattern, "");
    if (generated) for (const pattern of generatedAllowed) remainder = remainder.replace(pattern, "");
    if (retired_namespace.test(remainder)) failures.push(`${label}:${index + 1}`);
  });
}

const tracked = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"], { cwd: repo })
  .toString("utf8")
  .split("\0")
  .filter((path) => path && existsSync(join(repo, path)));
for (const path of tracked) {
  const name = path.split("/").at(-1);
  if (content_exempt_names.has(name) || content_exempt_prefixes.some((prefix) => path.startsWith(prefix))) continue;
  inspect(join(repo, path), path);
}

function inspectTree(relative) {
  const root = join(repo, relative);
  if (!existsSync(root)) return;
  for (const entry of readdirSync(root)) {
    if (content_exempt_names.has(entry)) continue;
    const path = join(root, entry);
    const label = join(relative, entry).replaceAll("\\", "/");
    if (statSync(path).isDirectory()) inspectTree(label);
    else inspect(path, label, true);
  }
}
for (const path of generated) inspectTree(path);

if (failures.length) {
  console.error(`Retired namespace found:\n${failures.join("\n")}`);
  process.exit(1);
}
console.log(`namespace guard OK: ${tracked.length} tracked files`);
