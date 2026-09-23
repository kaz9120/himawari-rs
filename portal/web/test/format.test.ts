import { expect, test } from "bun:test";
import { formatAgo, formatCount, formatElo, formatEta, formatRate, parseTime, percent } from "../src/format";

test("残り時間は分・時間・日で丸める", () => {
  expect(formatEta(null)).toBe("");
  expect(formatEta(0)).toBe("まもなく");
  expect(formatEta(90)).toBe("残り約2分");
  expect(formatEta(5 * 3600)).toBe("残り約5.0時間");
  // 付け直し（ADR-0219）の実測に近い値。216時間は日で読む
  expect(formatEta(216 * 3600)).toBe("残り約9.0日");
});

test("大きな数は億・万で読む", () => {
  expect(formatCount(2_000_000_000)).toBe("20億");
  expect(formatCount(129_826_816)).toBe("1.3億");
  expect(formatCount(122_071)).toBe("12.2万");
  expect(formatCount(8000)).toBe("8,000");
  expect(formatCount(null)).toBe("?");
});

test("hmwrの +0900 の時刻を読める", () => {
  expect(parseTime("2026-09-23T01:40:16+0900")).toBe(Date.parse("2026-09-22T16:40:16Z"));
  expect(parseTime("2026-09-22T04:49:50Z")).toBe(Date.parse("2026-09-22T04:49:50Z"));
  expect(parseTime("t0")).toBeNull();
  const now = new Date("2026-09-23T02:40:16+09:00");
  expect(formatAgo("2026-09-23T01:40:16+0900", now)).toBe("1時間前");
});

test("Eloは符号を必ず付ける", () => {
  expect(formatElo("+99.4")).toBe("+99.4");
  expect(formatElo(-23.8)).toBe("−23.8");
  expect(formatElo("-0.0")).toBe("±0.0");
});

test("速さと割合", () => {
  expect(formatRate(2403.2, "局面")).toBe("2,403局面/秒");
  expect(formatRate(null)).toBe("");
  expect(percent(50, 200)).toBe(25);
  expect(percent(5, null)).toBeNull();
});
