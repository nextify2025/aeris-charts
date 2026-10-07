import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

export function crop_png(source, x, y, width, height) {
  const output = new PNG({ width, height });
  PNG.bitblt(source, output, x, y, width, height, 0, 0);
  return output;
}

export function count_different(a, b) {
  if (a.width !== b.width || a.height !== b.height) throw new Error("parity images have different dimensions");
  return pixelmatch(a.data, b.data, new PNG({ width: a.width, height: a.height }).data, a.width, a.height, {
    threshold: 0,
    includeAA: true,
  });
}

export function max_channel_delta(a, b) {
  if (a.width !== b.width || a.height !== b.height) throw new Error("parity images have different dimensions");
  let max_delta = 0;
  for (let i = 0; i < a.data.length; i += 1) {
    max_delta = Math.max(max_delta, Math.abs(a.data[i] - b.data[i]));
  }
  return max_delta;
}
