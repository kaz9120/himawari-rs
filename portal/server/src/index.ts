/**
 * ポータルの起動点。HTTPとWebSocketを1プロセスで出す。
 * portal で `bun run start`（開発中は `bun run dev:server`）。
 */
import { join } from "node:path";
import { AccessVerifier } from "./access";
import { createApp } from "./app";
import { loadConfig } from "./config";
import { hmwrRunner, StatusPoller } from "./poller";

const config = loadConfig();

const poller = new StatusPoller(hmwrRunner(config.repo), (snapshot) => {
  app.broadcast(snapshot);
  if (snapshot.error) console.error(`[status] ${snapshot.error}`);
});

const app = createApp({
  snapshot: () => poller.get(),
  refresh: () => poller.refresh(),
  verifier: config.access ? new AccessVerifier(config.access) : null,
  webDist: join(import.meta.dir, "..", "..", "web", "dist"),
});

const server = Bun.serve({
  port: config.port,
  hostname: config.host,
  fetch: (request, bunServer) => app.fetch(request, bunServer),
  websocket: app.websocket,
});
app.attach(server);
poller.start(config.intervalSec);

console.log(
  `himawari portal: http://${server.hostname}:${server.port}  (repo: ${config.repo})  ` +
    (config.access ? "Cloudflare Access: 有効" : "Cloudflare Access: 未設定（localhostのみ）"),
);

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    poller.stop();
    process.exit(0);
  });
}
