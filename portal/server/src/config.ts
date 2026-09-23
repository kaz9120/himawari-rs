/**
 * 設定は環境変数で渡す。launchdのplistに書く前提で、設定ファイルは持たない。
 */
import { resolve } from "node:path";

export interface AccessConfig {
  teamDomain: string;
  aud: string;
}

export interface PortalConfig {
  port: number;
  /** 待ち受けアドレス。127.0.0.1なら、外からはCloudflare Tunnelだけが入口になる */
  host: string;
  /** Cloudflare Accessの検証。未設定なら、localhost以外からの要求をすべて断る */
  access: AccessConfig | null;
  /** hmwr status --json を読む間隔（秒） */
  intervalSec: number;
  /** リポジトリのルート。bin/hmwr の場所を決める */
  repo: string;
}

export const DEFAULT_PORT = 8790;
export const DEFAULT_HOST = "127.0.0.1";
export const DEFAULT_INTERVAL_SEC = 30;

/** portal/server/src から3つ上がリポジトリのルート */
export const REPO_ROOT = resolve(import.meta.dir, "..", "..", "..");

function positiveInt(value: string | undefined, fallback: number, name: string): number {
  if (value === undefined || value === "") return fallback;
  const n = Number(value);
  if (!Number.isInteger(n) || n <= 0) throw new Error(`${name} は正の整数で書く: ${value}`);
  return n;
}

/**
 * 環境変数から設定を読む。
 *
 * Accessの2つは両方そろったときだけ有効にする。片方だけなら起動を止める。
 * 書きかけの設定のまま、検証なしで公開してしまうのを防ぐためである。
 */
export function loadConfig(env: Record<string, string | undefined> = process.env): PortalConfig {
  const teamDomain = (env.HMWR_ACCESS_TEAM_DOMAIN ?? "").trim();
  const aud = (env.HMWR_ACCESS_AUD ?? "").trim();
  if ((teamDomain === "") !== (aud === "")) {
    throw new Error("HMWR_ACCESS_TEAM_DOMAIN と HMWR_ACCESS_AUD は両方そろえて書く（片方だけでは検証できない）");
  }
  return {
    port: positiveInt(env.HMWR_PORTAL_PORT, DEFAULT_PORT, "HMWR_PORTAL_PORT"),
    host: env.HMWR_PORTAL_HOST?.trim() || DEFAULT_HOST,
    access: teamDomain ? { teamDomain, aud } : null,
    intervalSec: positiveInt(env.HMWR_PORTAL_INTERVAL, DEFAULT_INTERVAL_SEC, "HMWR_PORTAL_INTERVAL"),
    repo: env.HMWR_REPO?.trim() || REPO_ROOT,
  };
}
