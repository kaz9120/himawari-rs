/** 状態の値を画面の言葉へ対応させる */
import type { RunState, StepState } from "@himawari-portal/shared";
import type { Tone } from "./components/ui";

export const KIND_LABEL: Record<string, string> = {
  relabel: "付け直し",
  train: "学習",
  match: "対局",
  exp: "実験",
  spsa: "SPSA",
};

export function kindLabel(kind: string): string {
  return KIND_LABEL[kind] ?? kind;
}

export const RUN_STATE: Record<RunState, { label: string; tone: Tone }> = {
  running: { label: "実行中", tone: "amber" },
  done: { label: "完了", tone: "green" },
  failed: { label: "失敗", tone: "red" },
  stopped: { label: "停止", tone: "stone" },
};

export const STEP_STATE: Record<StepState, { mark: string; cls: string }> = {
  done: { mark: "✓", cls: "text-emerald-600 dark:text-emerald-400" },
  running: { mark: "▶", cls: "text-amber-600 dark:text-amber-400" },
  pending: { mark: "・", cls: "text-stone-400" },
  failed: { mark: "✕", cls: "text-rose-600 dark:text-rose-400" },
};

/** 対局の判定（hmwr/sprt_log.py の EXIT_BY_VERDICT のキー） */
export function decisionTone(decision: string | undefined): Tone {
  switch (decision) {
    case "H1":
      return "green";
    case "H0":
      return "red";
    case "見送り":
    case "打ち切り":
      return "amber";
    case "指し切り":
      return "blue";
    default:
      return "stone";
  }
}
