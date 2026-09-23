/**
 * serverの検証。入口の守りを最も重く見る（ADR-0220の決定1）。
 *
 * - localhostの免除は、接続元とHostの両方がそろったときだけ効く
 * - Accessが未設定なら、localhost以外は断る
 * - Accessが設定されていれば、正しいJWTだけを通す
 * - hmwr statusが失敗しても、前回の状態は残る
 */
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Server as BunServerOf } from "bun";
import { exportJWK, generateKeyPair, SignJWT } from "jose";
import type { Snapshot } from "@himawari-portal/shared";
import { AccessVerifier, isLocalExempt } from "../src/access";
import { createApp } from "../src/app";
import { loadConfig } from "../src/config";
import { parseStatus, StatusPoller } from "../src/poller";

type Server = BunServerOf<undefined>;

const STATUS = {
  generated: "2026-09-23T17:00:00+0900",
  queue: { paused: false, running: [], queued: [], failed: [] },
  heartbeats: [],
  results: { matches: [], nets: [] },
  resources: { disk_free_gb: 68.8, disk_total_gb: 994.7, launchd: { loaded: true, pid: 1 }, open_prs: [] },
  config: [["評価関数", "data/nets/x.hmwr"]],
};

const snapshot: Snapshot = { status: parseStatus(JSON.stringify(STATUS)), fetchedAt: "t", error: null };

/** requestIP だけを持つ偽のserver */
function fakeServer(ip: string): Server {
  return { requestIP: () => ({ address: ip, family: "IPv4", port: 1 }), upgrade: () => false } as unknown as Server;
}

function request(path: string, host: string, headers: Record<string, string> = {}): Request {
  return new Request(`http://${host}${path}`, { headers: { host, ...headers } });
}

const webDist = mkdtempSync(join(tmpdir(), "portal-dist-"));
writeFileSync(join(webDist, "index.html"), "<html>portal</html>");

function app(verifier: Parameters<typeof createApp>[0]["verifier"]) {
  return createApp({ snapshot: () => snapshot, refresh: async () => snapshot, verifier, webDist });
}

describe("localhostの免除", () => {
  test("接続元とHostがともにlocalhostなら免除する", () => {
    expect(isLocalExempt(request("/", "localhost:8790"), "127.0.0.1")).toBe(true);
    expect(isLocalExempt(request("/", "127.0.0.1:8790"), "::ffff:127.0.0.1")).toBe(true);
  });

  test("Tunnel越し（接続元はloopback、Hostは公開名）は免除しない", () => {
    expect(isLocalExempt(request("/", "portal.example.com"), "127.0.0.1")).toBe(false);
  });

  test("LANの端末からは免除しない", () => {
    expect(isLocalExempt(request("/", "localhost:8790"), "192.168.1.5")).toBe(false);
  });
});

describe("Accessが未設定のとき", () => {
  const a = app(null);

  test("localhostからは読める", async () => {
    const res = await a.fetch(request("/api/status", "localhost:8790"), fakeServer("127.0.0.1"));
    expect(res?.status).toBe(200);
    expect(((await res?.json()) as Snapshot).status?.resources.disk_free_gb).toBe(68.8);
  });

  test("Tunnel越しは静的配信も含めて断る", async () => {
    for (const path of ["/api/status", "/", "/ws"]) {
      const res = await a.fetch(request(path, "portal.example.com"), fakeServer("127.0.0.1"));
      expect(res?.status).toBe(403);
    }
  });
});

describe("Accessが設定されているとき", () => {
  let jwks: Server;
  let privateKey: CryptoKey;
  let verifier: AccessVerifier;
  const aud = "test-aud";

  beforeAll(async () => {
    const pair = await generateKeyPair("RS256");
    privateKey = pair.privateKey;
    const jwk = { ...(await exportJWK(pair.publicKey)), kid: "k1", alg: "RS256" };
    jwks = Bun.serve({ port: 0, fetch: () => Response.json({ keys: [jwk] }) });
    verifier = new AccessVerifier({ teamDomain: "team.example.com", aud }, `http://localhost:${jwks.port}/certs`);
  });

  afterAll(() => {
    jwks.stop(true);
  });

  const sign = (audience: string, expSec = 60) =>
    new SignJWT({ sub: "owner" })
      .setProtectedHeader({ alg: "RS256", kid: "k1" })
      .setAudience(audience)
      .setIssuedAt()
      .setExpirationTime(Math.floor(Date.now() / 1000) + expSec)
      .sign(privateKey);

  test("正しいJWTなら通す（ヘッダでもCookieでも）", async () => {
    const a = app(verifier);
    const token = await sign(aud);
    const byHeader = await a.fetch(
      request("/api/status", "portal.example.com", { "cf-access-jwt-assertion": token }),
      fakeServer("127.0.0.1"),
    );
    expect(byHeader?.status).toBe(200);
    const byCookie = await a.fetch(
      request("/", "portal.example.com", { cookie: `other=1; CF_Authorization=${token}` }),
      fakeServer("127.0.0.1"),
    );
    expect(byCookie?.status).toBe(200);
    expect(await byCookie?.text()).toContain("portal");
  });

  test("JWTがない・audが違う・期限切れなら断る", async () => {
    const a = app(verifier);
    const cases: Record<string, string>[] = [
      {},
      { "cf-access-jwt-assertion": await sign("other-aud") },
      { "cf-access-jwt-assertion": await sign(aud, -10) },
    ];
    for (const headers of cases) {
      const res = await a.fetch(request("/api/status", "portal.example.com", headers), fakeServer("127.0.0.1"));
      expect(res?.status).toBe(401);
    }
  });
});

describe("設定", () => {
  test("Accessの2つの片方だけでは起動しない", () => {
    expect(() => loadConfig({ HMWR_ACCESS_TEAM_DOMAIN: "team.example.com" })).toThrow();
    expect(loadConfig({}).access).toBeNull();
    expect(loadConfig({}).host).toBe("127.0.0.1");
  });
});

describe("状態の取得", () => {
  test("失敗しても前回の状態を残し、理由だけを差し替える", async () => {
    const outputs = [
      { code: 0, stdout: JSON.stringify(STATUS), stderr: "" },
      { code: 3, stdout: "", stderr: "ghが落ちた\n詳細" },
      { code: 0, stdout: "{壊れたJSON", stderr: "" },
    ];
    const seen: Snapshot[] = [];
    const poller = new StatusPoller(async () => outputs.shift()!, (s) => seen.push(s));

    const first = await poller.refresh();
    expect(first.error).toBeNull();
    expect(first.status?.resources.disk_free_gb).toBe(68.8);

    const second = await poller.refresh();
    expect(second.error).toContain("ghが落ちた");
    expect(second.status).toEqual(first.status);

    const third = await poller.refresh();
    expect(third.error).toContain("読めない");
    expect(third.status).toEqual(first.status);
    expect(seen).toHaveLength(3);
  });

  test("読んでいる最中の呼び出しは、同じ取得にまとめる", async () => {
    let calls = 0;
    const poller = new StatusPoller(async () => {
      calls += 1;
      await Bun.sleep(5);
      return { code: 0, stdout: JSON.stringify(STATUS), stderr: "" };
    });
    await Promise.all([poller.refresh(), poller.refresh(), poller.refresh()]);
    expect(calls).toBe(1);
  });

  test("形が崩れたJSONは受け取らない", () => {
    expect(() => parseStatus(JSON.stringify({ ...STATUS, heartbeats: undefined }))).toThrow();
  });
});
