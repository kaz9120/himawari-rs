/**
 * 表示の整形。数値を画面で読みやすい形にするだけで、状態の解釈はしない。
 * 純粋な関数だけを置き、test/format.test.ts で確かめる。
 */

/** 残り時間。1時間未満は分、2日未満は時間、それ以上は日で出す */
export function formatEta(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return "";
  if (seconds <= 0) return "まもなく";
  if (seconds < 3600) return `残り約${Math.max(1, Math.round(seconds / 60))}分`;
  if (seconds < 48 * 3600) return `残り約${(seconds / 3600).toFixed(1)}時間`;
  return `残り約${(seconds / 86400).toFixed(1)}日`;
}

/** 所要時間。ステップの所要などに使う */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return "";
  if (seconds < 60) return `${Math.round(seconds)}秒`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}分`;
  if (seconds < 48 * 3600) return `${(seconds / 3600).toFixed(1)}時間`;
  return `${(seconds / 86400).toFixed(1)}日`;
}

/** 大きな数を億・万で丸める。1万未満はそのまま区切りを入れる */
export function formatCount(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return "?";
  const abs = Math.abs(n);
  if (abs >= 1e8) return `${trim(n / 1e8)}億`;
  if (abs >= 1e4) return `${trim(n / 1e4)}万`;
  return n.toLocaleString("ja-JP");
}

function trim(x: number): string {
  const digits = Math.abs(x) >= 100 ? 0 : Math.abs(x) >= 10 ? 1 : 2;
  return x.toFixed(digits).replace(/\.?0+$/, "");
}

/** 速さ。1秒あたりの件数 */
export function formatRate(rate: number | null | undefined, unit = ""): string {
  if (rate === null || rate === undefined || !Number.isFinite(rate) || rate <= 0) return "";
  const shown = rate >= 100 ? Math.round(rate).toLocaleString("ja-JP") : rate.toFixed(2).replace(/\.?0+$/, "");
  return `${shown}${unit}/秒`;
}

/** 進み具合の割合（0〜100）。総量が分からなければnull */
export function percent(done: number | null | undefined, total: number | null | undefined): number | null {
  if (done === null || done === undefined || !total) return null;
  return Math.min(100, Math.max(0, (done / total) * 100));
}

/** いつ頃か。「3分前」の形。未来の時刻は「たった今」に寄せる */
export function formatAgo(iso: string | null | undefined, now: Date = new Date()): string {
  const t = parseTime(iso);
  if (t === null) return "";
  const sec = Math.max(0, (now.getTime() - t) / 1000);
  if (sec < 60) return "たった今";
  if (sec < 3600) return `${Math.floor(sec / 60)}分前`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}時間前`;
  return `${Math.floor(sec / 86400)}日前`;
}

/**
 * hmwrの時刻を読む。Pythonの %z は +0900 の形で出すので、JSが読める +09:00 へ直す。
 */
export function parseTime(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const fixed = iso.replace(/([+-]\d{2})(\d{2})$/, "$1:$2");
  const t = Date.parse(fixed);
  return Number.isNaN(t) ? null : t;
}

/** Eloの表示。符号を必ず付ける */
export function formatElo(elo: string | number | null | undefined): string {
  if (elo === null || elo === undefined || elo === "") return "";
  const n = typeof elo === "number" ? elo : Number(elo);
  if (!Number.isFinite(n)) return String(elo);
  return `${n > 0 ? "+" : n < 0 ? "−" : "±"}${Math.abs(n).toFixed(1)}`;
}
