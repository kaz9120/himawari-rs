/**
 * 「結果」の画面。直近の対局の判定と、直近の学習の検証損失を出す。
 * 採否は対局で決めるので、対局を先に並べる（CLAUDE.mdの「学習の測定」）。
 */
import type { MatchResult, NetResult, Status } from "@himawari-portal/shared";
import { formatAgo, formatDuration, formatElo } from "../format";
import { decisionTone } from "../labels";
import { Badge, Card, Empty, Section } from "./ui";

export function ResultsView({ status, now }: { status: Status; now: Date }) {
  const { matches, nets } = status.results;
  return (
    <div className="space-y-6">
      <Section title="対局">
        {matches.length === 0 && <Empty>結果はまだありません。</Empty>}
        <Card className="divide-y divide-stone-100 p-0 dark:divide-stone-800">
          {matches.map((m) => (
            <MatchRow key={m.name} m={m} now={now} />
          ))}
        </Card>
      </Section>

      <Section title="学習">
        {nets.length === 0 && <Empty>学習の記録はまだありません。</Empty>}
        <Card className="divide-y divide-stone-100 p-0 dark:divide-stone-800">
          {nets.map((n, i) => (
            <NetRow key={`${n.name}-${i}`} n={n} />
          ))}
        </Card>
      </Section>
    </div>
  );
}

function MatchRow({ m, now }: { m: MatchResult; now: Date }) {
  return (
    <div className="space-y-1 px-4 py-3">
      <div className="flex items-center gap-2">
        <Badge tone={decisionTone(m.decision)}>{m.decision ?? "?"}</Badge>
        <span className="min-w-0 truncate font-medium">{m.name}</span>
      </div>
      <div className="num flex flex-wrap items-baseline gap-x-3 text-sm">
        <span className="font-semibold">{formatElo(m.elo)}</span>
        {m.ci_low !== undefined && (
          <span className="text-stone-500">
            [{formatElo(m.ci_low)}, {formatElo(m.ci_high)}]
          </span>
        )}
        {m.games && <span className="text-stone-500">{Number(m.games).toLocaleString("ja-JP")}局</span>}
        {m.llr && <span className="text-stone-500">LLR {m.llr}</span>}
        <span className="ml-auto text-xs text-stone-400">{formatAgo(m.finished_at, now)}</span>
      </div>
    </div>
  );
}

function NetRow({ n }: { n: NetResult }) {
  return (
    <div className="space-y-1 px-4 py-3">
      <div className="flex items-baseline gap-2">
        <span className="min-w-0 truncate font-medium">{n.name}</span>
        <span className="num ml-auto shrink-0 text-sm">
          <span className="text-stone-500">検証損失 </span>
          <span className="font-semibold">{n.best_valid}</span>
        </span>
      </div>
      <div className="num flex flex-wrap gap-x-3 text-xs text-stone-500">
        <span>{n.timestamp?.slice(0, 10)}</span>
        {n.elapsed_s && <span>{formatDuration(Number(n.elapsed_s))}</span>}
        {n.data_n && <span>{Number(n.data_n).toLocaleString("ja-JP")}局面</span>}
      </div>
      {n.notes && <p className="text-xs text-stone-500">{n.notes}</p>}
    </div>
  );
}
