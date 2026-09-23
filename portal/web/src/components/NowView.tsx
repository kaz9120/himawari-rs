/**
 * 「今」の画面。走っている実験とそのステップ、各処理の進み具合を出す。
 * 待っている実験と失敗した実験も、読むだけの形でここに並べる（操作は5段目）。
 */
import type { Heartbeat, QueueItem, Status } from "@himawari-portal/shared";
import { formatAgo, formatCount, formatDuration, formatElo, formatEta, formatRate, percent } from "../format";
import { kindLabel, RUN_STATE, STEP_STATE } from "../labels";
import { Badge, Card, Empty, Facts, Notice, ProgressBar, Section } from "./ui";

/** 終わった処理は、新しいものからこの件数だけ出す */
const RECENT_FINISHED = 5;

export function NowView({ status, now }: { status: Status; now: Date }) {
  const { queue, heartbeats } = status;
  const running = heartbeats.filter((b) => b.state === "running");
  const finished = heartbeats.filter((b) => b.state !== "running").slice(0, RECENT_FINISHED);

  return (
    <div className="space-y-6">
      {queue.paused && <Notice tone="amber">キューは一時停止中です。走っているステップが終わると、次は始めません。</Notice>}
      {queue.error && <Notice tone="amber">キューを読めませんでした: {queue.error}</Notice>}

      <Section title="実験">
        {queue.running.length === 0 && <Empty>走っている実験はありません。</Empty>}
        {queue.running.map((item) => (
          <ExperimentCard key={item.number} item={item} />
        ))}
      </Section>

      <Section title="進み具合">
        {running.length === 0 && <Empty>実行中の処理はありません。</Empty>}
        {running.map((b) => (
          <HeartbeatCard key={`${b.kind}-${b.name}`} beat={b} now={now} />
        ))}
      </Section>

      {(queue.queued.length > 0 || queue.failed.length > 0) && (
        <Section title="待ち・失敗">
          <Card className="divide-y divide-stone-100 p-0 dark:divide-stone-800">
            {queue.queued.map((item) => (
              <QueueRow key={item.number} item={item} badge={<Badge>待ち</Badge>} />
            ))}
            {queue.failed.map((item) => (
              <QueueRow key={item.number} item={item} badge={<Badge tone="red">失敗</Badge>} />
            ))}
          </Card>
        </Section>
      )}

      {finished.length > 0 && (
        <Section title="最近終わった処理">
          {finished.map((b) => (
            <HeartbeatCard key={`${b.kind}-${b.name}`} beat={b} now={now} compact />
          ))}
        </Section>
      )}
    </div>
  );
}

function ExperimentCard({ item }: { item: QueueItem }) {
  const steps = item.steps ?? [];
  const done = steps.filter((s) => s.state === "done").length;
  return (
    <Card>
      <div className="mb-3 space-y-1">
        <div className="flex items-center gap-2 text-xs text-stone-500 dark:text-stone-400">
          <a className="underline-offset-2 hover:underline" href={issueUrl(item.number)} target="_blank" rel="noreferrer">
            #{item.number}
          </a>
          <span className="truncate">{item.spec}</span>
          <span className="ml-auto num">
            {done}/{steps.length}
          </span>
        </div>
        <h3 className="font-semibold leading-snug">{item.title.replace(/^実験:\s*/, "")}</h3>
      </div>
      <ol className="space-y-1.5">
        {steps.map((step) => {
          const s = STEP_STATE[step.state];
          return (
            <li key={step.id} className="flex items-baseline gap-2 text-sm">
              <span className={`w-4 shrink-0 text-center ${s.cls}`}>{s.mark}</span>
              <span className={step.state === "running" ? "font-semibold" : step.state === "pending" ? "text-stone-500" : ""}>
                {step.id}
              </span>
              {step.state === "done" && (
                <span className="num ml-auto shrink-0 text-xs text-stone-500">{formatDuration(step.seconds)}</span>
              )}
              {step.error && <span className="ml-auto text-xs text-rose-600">{step.error}</span>}
            </li>
          );
        })}
      </ol>
    </Card>
  );
}

function HeartbeatCard({ beat, now, compact = false }: { beat: Heartbeat; now: Date; compact?: boolean }) {
  const p = beat.progress;
  const pct = percent(p?.done, p?.total);
  const state = RUN_STATE[beat.state];
  const problems: string[] = [];
  if (beat.state === "running" && beat.alive === false) problems.push("プロセスが見つかりません");
  if (beat.stale) problems.push("更新が途絶えています");

  return (
    <Card className={compact ? "py-3" : ""}>
      <div className="flex items-center gap-2">
        <Badge tone={state.tone}>{state.label}</Badge>
        <span className="text-xs text-stone-500 dark:text-stone-400">{kindLabel(beat.kind)}</span>
        <span className="min-w-0 truncate font-medium">{beat.name}</span>
      </div>

      {!compact && (
        <div className="mt-3 space-y-1.5">
          <ProgressBar value={pct} />
          <div className="num flex flex-wrap items-baseline gap-x-3 text-sm">
            {pct !== null && <span className="font-semibold">{pct.toFixed(1)}%</span>}
            <span className="text-stone-600 dark:text-stone-300">
              {formatCount(p?.done)}
              {p?.total ? ` / ${formatCount(p.total)}` : ""}
              {p?.unit ?? ""}
            </span>
            <span className="text-stone-500">{formatRate(beat.rate)}</span>
            <span className="ml-auto font-medium text-amber-700 dark:text-amber-400">{formatEta(beat.eta_seconds)}</span>
          </div>
        </div>
      )}

      <div className="mt-3">
        <Facts items={detailFacts(beat)} />
      </div>

      {problems.length > 0 && (
        <div className="mt-3">
          <Notice tone="red">{problems.join("。")}</Notice>
        </div>
      )}
      <p className="mt-2 text-xs text-stone-400">更新 {formatAgo(beat.updated, now)}</p>
    </Card>
  );
}

/** 領域ごとに、見たい追加情報を選んで並べる */
function detailFacts(beat: Heartbeat): [string, string][] {
  const d = beat.detail ?? {};
  const str = (v: unknown) => (v === null || v === undefined ? "" : String(v));
  const num = (v: unknown, digits: number) => (typeof v === "number" ? v.toFixed(digits) : str(v));
  switch (beat.kind) {
    case "match":
      return [
        ["判定", str(d.decision)],
        ["Elo", d.elo !== undefined ? `${formatElo(d.elo as number)} [${formatElo(d.ci_low as number)}, ${formatElo(d.ci_high as number)}]` : ""],
        ["LLR", num(d.llr, 2)],
        ["勝敗", str(d.wdl)],
        ["止め方", str(d.stop)],
      ];
    case "train":
      return [
        ["loss", num(d.loss, 5)],
        ["検証損失", num(d.valid, 5)],
        ["最良", num(d.best_valid, 5)],
        ["教師データ", str(d.data)],
      ];
    case "relabel":
      return [
        ["ラベラー", str(d.labeler)],
        ["スケール", str(d.scale)],
      ];
    case "exp":
      return [["ステップ", str(d.step)]];
    case "spsa":
      return [["動いた項目", d.moved !== undefined ? `${str(d.moved)}/${str(d.params)}` : ""]];
    default:
      return Object.entries(d).map(([k, v]) => [k, str(v)]);
  }
}

function QueueRow({ item, badge }: { item: QueueItem; badge: React.ReactNode }) {
  return (
    <a
      className="flex items-center gap-2 px-4 py-3 text-sm hover:bg-stone-50 dark:hover:bg-stone-800/50"
      href={issueUrl(item.number)}
      target="_blank"
      rel="noreferrer"
    >
      {badge}
      <span className="num text-stone-500">#{item.number}</span>
      <span className="min-w-0 truncate">{item.title.replace(/^実験:\s*/, "")}</span>
    </a>
  );
}

export function issueUrl(n: number): string {
  return `https://github.com/kaz9120/himawari-rs/issues/${n}`;
}
