import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

if (process.platform !== "win32" && process.platform !== "linux") {
  console.error("The GPUI/WebGPU pixel matrix reads the GPUI window through Windows DWM or the Linux X server and cannot run on this platform.");
  process.exit(1);
}
// docs/Architecture.md (验证) records that headless Chromium cannot present WebGPU frames
// on a Linux box without a GPU, and the GPUI window is read from an X server, so the Linux run is
// headed under a virtual display:
//   xvfb-run -a -s "-screen 0 2560x1600x24" bun run test:gpui-webgpu
if (process.platform === "linux" && !process.env.DISPLAY) {
  console.error("The Linux GPUI/WebGPU pixel matrix needs an X display; run it under `xvfb-run -a -s \"-screen 0 2560x1600x24\"`.");
  process.exit(1);
}

const cli = fileURLToPath(new URL("./node_modules/@playwright/test/cli.js", import.meta.url));
const result = spawnSync(
  process.execPath,
  [cli, "test", "tests/gpui-webgpu-matrix.spec.mjs", "--project=chromium", ...(process.platform === "linux" ? ["--headed"] : [])],
  {
    cwd: fileURLToPath(new URL(".", import.meta.url)),
    env: { ...process.env, AERIS_CHARTS_RUN_GPUI_WEBGPU_MATRIX: "1" },
    stdio: "inherit",
  },
);
if (result.error) throw result.error;
process.exit(result.status ?? 1);
