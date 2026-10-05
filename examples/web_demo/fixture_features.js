/** Shared feature fixtures consumed through each library's public API. */
export function marker_fixture(data) {
  const marker = (index, position, shape, color, text) => ({
    time: data[index].time,
    position,
    shape,
    color,
    text,
  });
  return [
    marker(840, "aboveBar", "arrowDown", "#f7525f", "SELL"),
    marker(880, "belowBar", "arrowUp", "#089981", "BUY"),
    marker(920, "inBar", "circle", "#7e57c2", "MID"),
    marker(960, "aboveBar", "square", "#2962ff", "NOTE"),
  ];
}

/**
 * Timeline-mark lane fixture (`?feature=timeline_marks`): two groups, a same-bar pair at bar 760,
 * a mark 30 minutes into bar 763 (it lands on that bar and folds with the pair once bars are
 * narrower than ~7 px), and one future mark (projected four slots past the last bar).
 */
export function timeline_mark_fixture(data) {
  const last = data[data.length - 1].time;
  const hour = 3600;
  return {
    groups: [
      { id: "earnings", label: "Earnings" },
      { id: "news", label: "News" },
    ],
    marks: [
      { id: "e1", time: data[700].time, group: "earnings", glyph: { shape: "circle", color: "#2962ff", letter: "E" }, title: "Q3 report" },
      { id: "e2", time: data[760].time, group: "earnings", glyph: { shape: "square", color: "#2962ff", letter: "E" }, title: "Guidance" },
      { id: "e3", time: data[760].time, group: "earnings", glyph: { shape: "circle", color: "#2962ff", letter: "E" }, title: "Call" },
      { id: "n1", time: data[763].time + 1800, group: "news", glyph: { shape: "diamond", color: "#f7525f", letter: "N" }, title: "Upgrade" },
      { id: "n2", time: data[900].time, group: "news", glyph: { shape: "pin", color: "#f7525f", letter: "N" }, title: "Merger" },
      { id: "f1", time: last + 4 * hour + 60, group: "earnings", glyph: { shape: "circle", color: "#089981", letter: "F" }, title: "Next report" },
    ],
  };
}

export function volume_fixture(data) {
  return data.map((bar) => ({
    time: bar.time,
    value: Math.round(500 + Math.abs(bar.close - bar.open) * 4000 + 300),
    color: bar.close >= bar.open ? "#08998180" : "#f7525f80",
  }));
}
