/** 画面の部品。見た目だけを持ち、状態の解釈はしない */
import type { ReactNode } from "react";

export function Card({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={`rounded-xl border border-stone-200 bg-white p-4 shadow-sm dark:border-stone-800 dark:bg-stone-900 ${className}`}
    >
      {children}
    </div>
  );
}

export function Section({ title, children, aside }: { title: string; children: ReactNode; aside?: ReactNode }) {
  return (
    <section className="space-y-2">
      <div className="flex items-baseline justify-between px-1">
        <h2 className="text-sm font-semibold text-stone-500 dark:text-stone-400">{title}</h2>
        {aside}
      </div>
      {children}
    </section>
  );
}

export type Tone = "green" | "red" | "amber" | "blue" | "stone";

const TONES: Record<Tone, string> = {
  green: "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/50 dark:text-emerald-300",
  red: "bg-rose-100 text-rose-800 dark:bg-rose-900/50 dark:text-rose-300",
  amber: "bg-amber-100 text-amber-800 dark:bg-amber-900/50 dark:text-amber-300",
  blue: "bg-sky-100 text-sky-800 dark:bg-sky-900/50 dark:text-sky-300",
  stone: "bg-stone-100 text-stone-700 dark:bg-stone-800 dark:text-stone-300",
};

export function Badge({ tone = "stone", children }: { tone?: Tone; children: ReactNode }) {
  return (
    <span className={`inline-flex shrink-0 items-center rounded-full px-2 py-0.5 text-xs font-medium ${TONES[tone]}`}>
      {children}
    </span>
  );
}

export function ProgressBar({ value, tone = "amber" }: { value: number | null; tone?: "amber" | "stone" }) {
  // 総量が分からないときは、縞で「進んでいるが割合は出せない」を示す
  if (value === null) {
    return (
      <div className="h-2 w-full overflow-hidden rounded-full bg-stone-200 dark:bg-stone-800">
        <div className="h-full w-full animate-pulse bg-stone-300 dark:bg-stone-700" />
      </div>
    );
  }
  const color = tone === "amber" ? "bg-amber-500" : "bg-stone-400";
  return (
    <div
      className="h-2 w-full overflow-hidden rounded-full bg-stone-200 dark:bg-stone-800"
      role="progressbar"
      aria-valuenow={Math.round(value)}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div className={`h-full rounded-full ${color}`} style={{ width: `${value}%` }} />
    </div>
  );
}

/** 項目と値の組を並べる */
export function Facts({ items }: { items: [string, ReactNode][] }) {
  const shown = items.filter(([, v]) => v !== "" && v !== null && v !== undefined);
  if (shown.length === 0) return null;
  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
      {shown.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="text-stone-500 dark:text-stone-400">{k}</dt>
          <dd className="num min-w-0 break-words">{v}</dd>
        </div>
      ))}
    </dl>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <p className="px-1 text-sm text-stone-500 dark:text-stone-400">{children}</p>;
}

export function Notice({ tone, children }: { tone: "amber" | "red"; children: ReactNode }) {
  const cls =
    tone === "red"
      ? "border-rose-300 bg-rose-50 text-rose-900 dark:border-rose-800 dark:bg-rose-950 dark:text-rose-200"
      : "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-200";
  return <div className={`rounded-lg border px-3 py-2 text-sm ${cls}`}>{children}</div>;
}
