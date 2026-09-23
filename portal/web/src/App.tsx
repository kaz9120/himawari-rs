/**
 * ポータルの外枠。見出しに接続と更新時刻を出し、下のタブで3つの画面を切り替える。
 * スマホで片手で見る前提で、タブは画面の下に置く（ADR-0220の「ポータル」）。
 */
import { useEffect, useState } from "react";
import { NowView } from "./components/NowView";
import { ResourcesView } from "./components/ResourcesView";
import { ResultsView } from "./components/ResultsView";
import { Notice } from "./components/ui";
import { formatAgo } from "./format";
import { useNow, useSnapshot, type Connection } from "./useSnapshot";

const TABS = [
  { id: "now", label: "今" },
  { id: "results", label: "結果" },
  { id: "resources", label: "資源" },
] as const;

type TabId = (typeof TABS)[number]["id"];

function tabFromHash(): TabId {
  const id = location.hash.replace(/^#/, "");
  return TABS.some((t) => t.id === id) ? (id as TabId) : "now";
}

export function App() {
  const { snapshot, connection, fetchError, refresh } = useSnapshot();
  const now = useNow();
  const [tab, setTab] = useState<TabId>(tabFromHash);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    const onHash = () => setTab(tabFromHash());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  const status = snapshot?.status ?? null;
  const onRefresh = async () => {
    setRefreshing(true);
    await refresh();
    setRefreshing(false);
  };

  return (
    <div className="mx-auto min-h-dvh max-w-2xl pb-24">
      <header className="sticky top-0 z-10 flex items-center gap-3 border-b border-stone-200 bg-stone-50/90 px-4 py-3 backdrop-blur dark:border-stone-800 dark:bg-stone-950/90">
        <h1 className="text-lg font-bold">🌻 himawari</h1>
        <ConnectionDot connection={connection} />
        <span className="ml-auto text-xs text-stone-500">
          {snapshot?.fetchedAt ? `更新 ${formatAgo(snapshot.fetchedAt, now)}` : "読み込み中"}
        </span>
        <button
          type="button"
          onClick={onRefresh}
          disabled={refreshing}
          className="rounded-md border border-stone-300 px-2 py-1 text-xs disabled:opacity-50 dark:border-stone-700"
        >
          {refreshing ? "取得中" : "再取得"}
        </button>
      </header>

      <main className="space-y-4 px-4 py-4">
        {fetchError && !status && <Notice tone="red">serverから状態を読めません: {fetchError}</Notice>}
        {snapshot?.error && (
          <Notice tone={status ? "amber" : "red"}>
            {status ? "最新の取得に失敗したので、前回の状態を出しています。" : ""}
            {snapshot.error}
          </Notice>
        )}
        {status && tab === "now" && <NowView status={status} now={now} />}
        {status && tab === "results" && <ResultsView status={status} now={now} />}
        {status && tab === "resources" && <ResourcesView status={status} />}
      </main>

      <nav className="fixed inset-x-0 bottom-0 z-10 border-t border-stone-200 bg-white/95 pb-[env(safe-area-inset-bottom)] backdrop-blur dark:border-stone-800 dark:bg-stone-900/95">
        <div className="mx-auto flex max-w-2xl">
          {TABS.map((t) => (
            <a
              key={t.id}
              href={`#${t.id}`}
              className={`flex-1 py-3 text-center text-sm font-medium ${
                tab === t.id ? "text-amber-600 dark:text-amber-400" : "text-stone-500"
              }`}
              aria-current={tab === t.id ? "page" : undefined}
            >
              {t.label}
            </a>
          ))}
        </div>
      </nav>
    </div>
  );
}

function ConnectionDot({ connection }: { connection: Connection }) {
  const [cls, label] =
    connection === "open"
      ? ["bg-emerald-500", "接続中"]
      : connection === "connecting"
        ? ["bg-amber-400", "接続しています"]
        : ["bg-rose-500", "切断。再接続します"];
  return (
    <span className="flex items-center gap-1 text-xs text-stone-500" title={label}>
      <span className={`h-2 w-2 rounded-full ${cls}`} />
      <span className="sr-only">{label}</span>
    </span>
  );
}
