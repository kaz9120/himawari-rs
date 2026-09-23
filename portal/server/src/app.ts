/**
 * Bun.serve に渡す fetch と websocket を組み立てる。起動は index.ts が担う。
 *
 * この段（ADR-0220の3段目）は読むだけである。状態の取得は StatusPoller に任せ、
 * ここは配ることと、入口を守ることだけを持つ。
 */
import { existsSync, statSync } from "node:fs";
import { join, normalize } from "node:path";
import type { Server as BunServerOf, ServerWebSocket, WebSocketHandler } from "bun";
import type { ServerMessage, Snapshot } from "@himawari-portal/shared";
import { isLocalExempt, type Verifier } from "./access";

const TOPIC = "status";

type Server = BunServerOf<undefined>;

export interface AppDeps {
  /** 最新の状態を返す */
  snapshot: () => Snapshot;
  /** 状態を今すぐ読み直す */
  refresh: () => Promise<Snapshot>;
  /** Accessの検証。nullなら、localhost以外からの要求をすべて断る */
  verifier: Verifier | null;
  /** web/dist の場所 */
  webDist: string;
}

export interface App {
  fetch: (request: Request, server?: Server) => Promise<Response | undefined>;
  websocket: WebSocketHandler<undefined>;
  attach: (server: Server) => void;
  broadcast: (snapshot: Snapshot) => void;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function error(message: string, status: number): Response {
  return json({ error: message }, status);
}

export function createApp(deps: AppDeps): App {
  let server: Server | null = null;

  const message = (snapshot: Snapshot): string =>
    JSON.stringify({ type: "snapshot", snapshot } satisfies ServerMessage);

  /**
   * 入口の守り。/api も /ws も静的配信も、必ずここを通す。
   *
   * Accessを設定していないときは、localhostからの要求しか通さない。pr_inboxは
   * 未設定なら検証を省くが、こちらは「Accessなしの入口を作らない」を
   * オーナーの決定（ADR-0220の決定1）としているので、閉じる側に倒す。
   */
  const guard = async (request: Request, target: Server | null): Promise<Response | null> => {
    const clientIp = target?.requestIP(request)?.address ?? null;
    if (isLocalExempt(request, clientIp)) return null;
    if (!deps.verifier) {
      return error("Cloudflare Accessが未設定なので、localhost以外からは開けません", 403);
    }
    const result = await deps.verifier.verify(request);
    return result.ok ? null : error(result.message, 401);
  };

  const serveStatic = (pathname: string): Response => {
    if (!existsSync(deps.webDist)) {
      return error("web/dist がありません。portal で bun run build を実行してください", 404);
    }
    const relative = normalize(pathname).replace(/^(\.\.[/\\])+/, "").replace(/^\/+/, "");
    const candidate = join(deps.webDist, relative);
    if (candidate.startsWith(deps.webDist) && existsSync(candidate) && statSync(candidate).isFile()) {
      return new Response(Bun.file(candidate));
    }
    // 1画面のアプリなので、静的ファイルに当たらないパスは index.html に寄せる
    const index = join(deps.webDist, "index.html");
    if (existsSync(index)) return new Response(Bun.file(index));
    return error("web/dist に index.html がありません", 404);
  };

  const handle = async (request: Request, passed?: Server): Promise<Response | undefined> => {
    const target = passed ?? server;
    const path = new URL(request.url).pathname;
    try {
      const denied = await guard(request, target);
      if (denied) return denied;

      if (path === "/ws") {
        // upgradeが通ったらResponseを返さない。返すと壊れた応答になる
        if (target?.upgrade(request)) return undefined;
        return error("WebSocketへのアップグレードが必要です", 426);
      }
      if (path === "/api/status") {
        if (request.method === "GET") return json(deps.snapshot());
        return error("そのメソッドは使えません", 405);
      }
      if (path === "/api/status/refresh") {
        if (request.method === "POST") return json(await deps.refresh());
        return error("そのメソッドは使えません", 405);
      }
      if (path.startsWith("/api/")) return error("そのエンドポイントはありません", 404);
      return serveStatic(path);
    } catch (err) {
      // Bunの既定の応答はJSONでなく、Cloudflare越しだと502になって理由が消える
      console.error(`[req] ${request.method} ${path} で例外:`, err);
      return error(`serverの内部エラー: ${err instanceof Error ? err.message : String(err)}`, 500);
    }
  };

  const websocket: WebSocketHandler<undefined> = {
    open(ws: ServerWebSocket<undefined>) {
      ws.subscribe(TOPIC);
      // 接続した直後に今の状態を送る。clientは最初のGETを省ける
      ws.send(message(deps.snapshot()));
    },
    message() {},
    close(ws: ServerWebSocket<undefined>) {
      ws.unsubscribe(TOPIC);
    },
  };

  return {
    fetch: handle,
    websocket,
    attach(instance) {
      server = instance;
    },
    broadcast(snapshot) {
      server?.publish(TOPIC, message(snapshot));
    },
  };
}
