import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export default function global_setup() {
  const repository_root = fileURLToPath(new URL("../..", import.meta.url));
  const args = ["build", "-p", "aeris_charts_native", "--example", "image_parity_fixture", "--example", "parity_fixture", "--locked"];
  const result = spawnSync("cargo", args, { cwd: repository_root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Native parity fixture build failed (cargo ${args.join(" ")}, exit ${result.status})`);
  }
}
