/**
 * `hmwr status --json` を定期的に読み、最新の状態を持つ。
 *
 * ポータルが状態を得る経路はこれだけである（ADR-0220の「状態の出力仕様」）。
 * 読めなかったときは前回の状態を残し、失敗の理由だけを差し替える。画面は
 * 「古いが見える」状態を保つ。
 */
import { join } from "node:path";
import type { Snapshot, Status } from "@himawari-portal/shared";

/** hmwr を呼び、標準出力を返す。テストで差し替える */
export type Runner = () => Promise<{ code: number; stdout: string; stderr: string }>;

/** 1回の取得の上限。GitHubが詰まっても次の周期を塞がない */
export const TIMEOUT_MS = 60_000;

export function hmwrRunner(repo: string): Runner {
  return async () => {
    const proc = Bun.spawn([join(repo, "bin", "hmwr"), "status", "--json"], {
      cwd: repo,
      stdout: "pipe",
      stderr: "pipe",
    });
    const timer = setTimeout(() => proc.kill(), TIMEOUT_MS);
    try {
      const [stdout, stderr, code] = await Promise.all([
        new Response(proc.stdout).text(),
        new Response(proc.stderr).text(),
        proc.exited,
      ]);
      return { code, stdout, stderr };
    } finally {
      clearTimeout(timer);
    }
  };
}

/** JSONの形を最低限だけ確かめる。形が崩れたら、画面を壊す前にここで落とす */
export function parseStatus(text: string): Status {
  const data = JSON.parse(text) as Partial<Status>;
  for (const key of ["generated", "queue", "heartbeats", "results", "resources", "config"] as const) {
    if (data[key] === undefined) throw new Error(`hmwr status --json に ${key} がない`);
  }
  if (!Array.isArray(data.heartbeats)) throw new Error("heartbeats が配列でない");
  return data as Status;
}

export class StatusPoller {
  private snapshot: Snapshot = { status: null, fetchedAt: null, error: null };
  private inflight: Promise<Snapshot> | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;

  constructor(
    private readonly runner: Runner,
    private readonly onUpdate: (snapshot: Snapshot) => void = () => {},
    private readonly now: () => Date = () => new Date(),
  ) {}

  get(): Snapshot {
    return this.snapshot;
  }

  /** 1回読む。読んでいる最中に呼ばれたら、同じ取得の結果を返す */
  refresh(): Promise<Snapshot> {
    this.inflight ??= this.fetchOnce().finally(() => {
      this.inflight = null;
    });
    return this.inflight;
  }

  start(intervalSec: number): void {
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), intervalSec * 1000);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  private async fetchOnce(): Promise<Snapshot> {
    let error: string | null = null;
    try {
      const { code, stdout, stderr } = await this.runner();
      if (code !== 0) {
        error = `hmwr status が終了コード ${code} で終わった: ${firstLine(stderr) || firstLine(stdout)}`;
      } else {
        this.snapshot = { status: parseStatus(stdout), fetchedAt: this.now().toISOString(), error: null };
      }
    } catch (err) {
      error = `hmwr status を読めない: ${err instanceof Error ? err.message : String(err)}`;
    }
    if (error) this.snapshot = { ...this.snapshot, error };
    this.onUpdate(this.snapshot);
    return this.snapshot;
  }
}

function firstLine(text: string): string {
  return text.trim().split("\n")[0] ?? "";
}
