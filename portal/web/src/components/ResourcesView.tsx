/**
 * 「資源」の画面。ディスクの空き、キューの常駐、開いているPR、設定を出す。
 */
import type { Status } from "@himawari-portal/shared";
import { Badge, Card, Empty, Facts, Notice, ProgressBar, Section } from "./ui";

/** 空きがこれを下回ったら警告する。教師データ1本が数百GBあるため */
const DISK_WARN_GB = 100;

export function ResourcesView({ status }: { status: Status }) {
  const r = status.resources;
  const used = r.disk_total_gb - r.disk_free_gb;
  const usedPct = r.disk_total_gb > 0 ? (used / r.disk_total_gb) * 100 : null;

  return (
    <div className="space-y-6">
      {r.error && <Notice tone="amber">GitHubを読めませんでした: {r.error}</Notice>}

      <Section title="ディスク（data/）">
        <Card className="space-y-2">
          <ProgressBar value={usedPct} tone={r.disk_free_gb < DISK_WARN_GB ? "amber" : "stone"} />
          <p className="num text-sm">
            空き <span className="font-semibold">{r.disk_free_gb.toFixed(1)} GB</span>
            <span className="text-stone-500"> / {r.disk_total_gb.toFixed(0)} GB</span>
          </p>
          {r.disk_free_gb < DISK_WARN_GB && (
            <Notice tone="amber">空きが{DISK_WARN_GB} GBを切っています。教師データを作る前に <code>hmwr clean</code> を検討してください。</Notice>
          )}
        </Card>
      </Section>

      <Section title="キューの常駐（launchd）">
        <Card>
          {r.launchd.loaded ? (
            <div className="flex items-center gap-2 text-sm">
              <Badge tone="green">動作中</Badge>
              {r.launchd.pid ? <span className="num text-stone-500">pid {r.launchd.pid}</span> : <span className="text-stone-500">待機中</span>}
            </div>
          ) : (
            <div className="flex items-center gap-2 text-sm">
              <Badge tone="red">停止</Badge>
              <span className="text-stone-500">キューが実験を拾いません</span>
            </div>
          )}
        </Card>
      </Section>

      <Section title="開いているPR">
        {r.open_prs.length === 0 ? (
          <Empty>ありません。</Empty>
        ) : (
          <Card className="divide-y divide-stone-100 p-0 dark:divide-stone-800">
            {r.open_prs.map((pr) => (
              <a
                key={pr.number}
                className="flex items-center gap-2 px-4 py-3 text-sm hover:bg-stone-50 dark:hover:bg-stone-800/50"
                href={`https://github.com/kaz9120/himawari-rs/pull/${pr.number}`}
                target="_blank"
                rel="noreferrer"
              >
                {pr.isDraft && <Badge>draft</Badge>}
                <span className="num text-stone-500">#{pr.number}</span>
                <span className="min-w-0 truncate">{pr.title}</span>
              </a>
            ))}
          </Card>
        )}
      </Section>

      <Section title="設定">
        <Card>
          <Facts items={status.config.map(([k, v]) => [k, v])} />
        </Card>
      </Section>
    </div>
  );
}
