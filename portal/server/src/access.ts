/**
 * Cloudflare AccessのJWTを検証する（ADR-0220の決定1）。
 *
 * 外からの入口はCloudflare Tunnelだけで、手前にAccessを置く。serverも
 * JWTを検証し、Accessを経ない要求を通さない。pr_inbox（~/Hobby/pr_inbox の
 * server/src/access.ts）と同じ守りで、同じ考え方をこちらへ移した。
 *
 * 免除は「同じMacのブラウザから localhost で開いた」ときだけである。
 * cloudflared も同じMacのlocalhostから繋ぐので、接続元だけを見ると
 * Tunnel越しの要求まで素通りする。Tunnel越しの要求はHostに公開ホスト名を
 * 載せるので、接続元とHostの両方で判定する。
 */
import { createRemoteJWKSet, jwtVerify } from "jose";
import type { AccessConfig } from "./config";

export const ACCESS_HEADER = "cf-access-jwt-assertion";
export const ACCESS_COOKIE = "CF_Authorization";
const JWKS_CACHE_MS = 60 * 60 * 1000;

export type AccessResult = { ok: true } | { ok: false; message: string };

/** Accessの検証の窓口。テストでは偽物に差し替える */
export interface Verifier {
  verify(request: Request): Promise<AccessResult>;
}

export function readCookie(header: string | null, name: string): string | null {
  if (!header) return null;
  for (const part of header.split(";")) {
    const trimmed = part.trim();
    const eq = trimmed.indexOf("=");
    if (eq <= 0 || trimmed.slice(0, eq) !== name) continue;
    return decodeURIComponent(trimmed.slice(eq + 1));
  }
  return null;
}

/** ヘッダを優先し、無ければCookieからJWTを取る */
export function readToken(request: Request): string | null {
  const header = request.headers.get(ACCESS_HEADER)?.trim();
  if (header) return header;
  return readCookie(request.headers.get("cookie"), ACCESS_COOKIE)?.trim() || null;
}

export function isLoopbackAddress(address: string | null | undefined): boolean {
  if (!address) return false;
  const bare = address.replace(/^\[|\]$/g, "");
  if (bare === "::1" || bare === "0:0:0:0:0:0:0:1") return true;
  const v4 = bare.startsWith("::ffff:") ? bare.slice("::ffff:".length) : bare;
  return /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(v4);
}

export function hostnameOf(request: Request): string {
  const host = request.headers.get("host") ?? "";
  if (host.startsWith("[")) return host.slice(0, host.indexOf("]") + 1).toLowerCase();
  const colon = host.indexOf(":");
  return (colon === -1 ? host : host.slice(0, colon)).toLowerCase();
}

/** 接続元がloopbackで、かつHostがlocalhostのときだけ免除する */
export function isLocalExempt(request: Request, clientIp: string | null | undefined): boolean {
  if (!isLoopbackAddress(clientIp)) return false;
  const host = hostnameOf(request);
  return host === "localhost" || host === "127.0.0.1" || host === "[::1]";
}

export class AccessVerifier implements Verifier {
  private readonly jwks: ReturnType<typeof createRemoteJWKSet>;

  constructor(
    private readonly config: AccessConfig,
    jwksUrl: string = `https://${config.teamDomain}/cdn-cgi/access/certs`,
  ) {
    this.jwks = createRemoteJWKSet(new URL(jwksUrl), {
      cacheMaxAge: JWKS_CACHE_MS,
      cooldownDuration: 10_000,
    });
  }

  async verify(request: Request): Promise<AccessResult> {
    const token = readToken(request);
    if (!token) return { ok: false, message: "Cloudflare Accessの認証が必要です" };
    try {
      await jwtVerify(token, this.jwks, {
        algorithms: ["RS256"],
        audience: this.config.aud,
        clockTolerance: 0,
      });
      return { ok: true };
    } catch (err) {
      return {
        ok: false,
        message: `Cloudflare Accessの認証に失敗しました: ${err instanceof Error ? err.message : String(err)}`,
      };
    }
  }
}
