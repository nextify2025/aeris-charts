import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

if (process.platform !== "win32" && process.platform !== "linux") {
  console.error("The GPUI/WebGPU pixel matrix reads the GPUI window through Windows DWM or the Linux X server and cannot run on this platform.");
  process.exit(1);
}
// docs/development/validation.md 记录了无 GPU 的 Linux 上无头 Chromium 的呈现限制。
// GPUI 窗口同样从 X 服务器读取，因此 Linux 在虚拟显示器下以有头模式运行：
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
