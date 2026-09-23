/**
 * `hmwr status --json` の形（ADR-0220）。
 *
 * 正はPythonの hmwr/commands/status.py と hmwr/heartbeat.py にある。ポータルは
 * このJSONだけを読み、ログやファイルを直接解釈しない。形を変えるときは、
 * Python側と同じPRでここも直す。
 */

export type RunState = "running" | "done" | "failed" | "stopped";

export interface Progress {
  done: number | null;
  total: number | null;
  unit: string;
}

/** 長く走るコマンドの状態ファイル（data/status/<領域>-<名前>.json） */
export interface Heartbeat {
  kind: string;
  name: string;
  state: RunState;
  progress?: Progress;
  rate?: number | null;
  eta_seconds?: number | null;
  started?: string | null;
  updated?: string | null;
  pid?: number;
  log?: string | null;
  /** 領域ごとの追加情報。対局ならElo・LLR、学習ならloss・valid */
  detail?: Record<string, unknown>;
  file?: string;
  /** 実行中のものだけに付く。プロセスが生きているか */
  alive?: boolean;
  /** 実行中のものだけに付く。更新が途絶えたか */
  stale?: boolean;
}

export type StepState = "done" | "running" | "pending" | "failed";

export interface Step {
  id: string;
  state: StepState;
  seconds?: number | null;
  finished?: string | null;
  run?: string;
  error?: string;
}

export interface QueueItem {
  number: number;
  title: string;
  spec: string | null;
  steps?: Step[];
}

export interface Queue {
  paused: boolean;
  running: QueueItem[];
  queued: QueueItem[];
  failed: QueueItem[];
  error?: string;
}

/** data/sprt/<名前>.result の要約。値は文字列のまま来る */
export interface MatchResult {
  name: string;
  decision?: string;
  elo?: string;
  ci_low?: string;
  ci_high?: string;
  games?: string;
  llr?: string;
  finished_at?: string;
}

/** training/runs/registry.tsv の1行。列は学習器が決める */
export type NetResult = Record<string, string>;

export interface OpenPr {
  number: number;
  title: string;
  isDraft: boolean;
  headRefName: string;
}

export interface Resources {
  disk_free_gb: number;
  disk_total_gb: number;
  launchd: { loaded: boolean; pid?: number | null };
  open_prs: OpenPr[];
  error?: string;
}

export interface Status {
  generated: string;
  queue: Queue;
  heartbeats: Heartbeat[];
  results: { matches: MatchResult[]; nets: NetResult[] };
  resources: Resources;
  /** [項目, 値] の組 */
  config: [string, string][];
}

/** serverが持つ最新の状態。取得に失敗しても、前回の状態は残す */
export interface Snapshot {
  status: Status | null;
  /** 最後に取得できた時刻（ISO） */
  fetchedAt: string | null;
  /** 最後の取得の失敗理由。成功すればnull */
  error: string | null;
}

export type ServerMessage = { type: "snapshot"; snapshot: Snapshot };
