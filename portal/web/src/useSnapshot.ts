/**
 * serverから最新の状態を受け取る。
 *
 * WebSocketで届くのを待ち、切れたら間隔を広げながら繋ぎ直す。繋がるまでの
 * 間も画面が空にならないよう、最初に1回GETする。
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { ServerMessage, Snapshot } from "@himawari-portal/shared";

export type Connection = "connecting" | "open" | "closed";

const RETRY_MIN_MS = 1000;
const RETRY_MAX_MS = 30_000;

export function useSnapshot() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [connection, setConnection] = useState<Connection>("connecting");
  const [fetchError, setFetchError] = useState<string | null>(null);
  const retry = useRef(RETRY_MIN_MS);

  const load = useCallback(async (path: string, init?: RequestInit) => {
    try {
      const res = await fetch(path, init);
      const body = (await res.json()) as Snapshot | { error: string };
      if (!res.ok || "error" in body && !("status" in body)) {
        setFetchError(("error" in body && body.error) || `HTTP ${res.status}`);
        return;
      }
      setSnapshot(body as Snapshot);
      setFetchError(null);
    } catch (err) {
      setFetchError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void load("/api/status");
    let ws: WebSocket | null = null;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let disposed = false;

    const connect = () => {
      setConnection("connecting");
      const scheme = location.protocol === "https:" ? "wss" : "ws";
      ws = new WebSocket(`${scheme}://${location.host}/ws`);
      ws.onopen = () => {
        retry.current = RETRY_MIN_MS;
        setConnection("open");
      };
      ws.onmessage = (event) => {
        const message = JSON.parse(String(event.data)) as ServerMessage;
        if (message.type === "snapshot") setSnapshot(message.snapshot);
      };
      ws.onclose = () => {
        if (disposed) return;
        setConnection("closed");
        timer = setTimeout(connect, retry.current);
        retry.current = Math.min(retry.current * 2, RETRY_MAX_MS);
      };
    };
    connect();

    return () => {
      disposed = true;
      if (timer) clearTimeout(timer);
      ws?.close();
    };
  }, [load]);

  const refresh = useCallback(() => load("/api/status/refresh", { method: "POST" }), [load]);

  return { snapshot, connection, fetchError, refresh };
}

/** 1秒ごとに再描画して、「N分前」を進める */
export function useNow(intervalMs = 15_000): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}
