// decky-api.mjs — talk to Decky Loader's local API (no sudo needed).
//
// The loader runs as root and owns ~/homebrew/plugins, but its HTTP/WS API on
// 127.0.0.1:1337 is reachable from the session user: GET /auth/token is
// unauthenticated on localhost and returns the X-Decky-Auth token, and the
// /ws websocket accepts the token as ?auth=. Through it we can ask the loader
// itself to install/uninstall a plugin zip (it unzips, chowns and imports the
// plugin as root — no systemctl restart, no sudo).
//
// Usage:
//   node decky-api.mjs install <plugin.zip> <plugin display name>
//   node decky-api.mjs uninstall <plugin display name>
//
// Requires Node >= 22 (global fetch and WebSocket). Protocol: JSON messages
// {"type":0,"route":...,"args":[...],"id":N} for calls, type 1 = reply,
// type -1 = error, type 5 = server event.

const BASE = "http://127.0.0.1:1337";
const [cmd, ...rest] = process.argv.slice(2);

async function main() {
  const token = (await (await fetch(`${BASE}/auth/token`)).text()).trim();
  if (!token) throw new Error("could not get auth token from loader (is it running?)");

  const ws = new WebSocket(`ws://127.0.0.1:1337/ws?auth=${encodeURIComponent(token)}`);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error("websocket connect failed"));
  });

  let nextId = 1;
  const pending = new Map();
  const call = (route, args) => {
    const id = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ type: 0, route, args, id }));
    });
  };

  const done = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("timed out waiting for the loader")), 60000);
    ws.onmessage = async (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.type === 1 && pending.has(msg.id)) {
        pending.get(msg.id).resolve(msg.result);
        pending.delete(msg.id);
      } else if (msg.type === -1 && pending.has(msg.id)) {
        pending.get(msg.id).reject(new Error(msg.error?.message ?? JSON.stringify(msg.error)));
        pending.delete(msg.id);
      } else if (msg.type === 5 && msg.event === "loader/add_plugin_install_prompt") {
        // install_plugin only queues the request; the actual install happens on confirm.
        const requestId = msg.args[2];
        try {
          await call("utilities/confirm_plugin_install", [requestId]);
        } catch (e) {
          clearTimeout(timer);
          reject(e);
          return;
        }
      } else if (msg.type === 5 && msg.event === "loader/plugin_download_finish") {
        clearTimeout(timer);
        resolve();
        return;
      }
    };
  });

  if (cmd === "install") {
    const [zip, name] = rest;
    if (!zip || !name) throw new Error("usage: install <plugin.zip> <plugin display name>");
    await call("utilities/install_plugin", [`file://${zip}`, name, "dev", "", 0]);
  } else if (cmd === "uninstall") {
    const [name] = rest;
    if (!name) throw new Error("usage: uninstall <plugin display name>");
    // uninstall replies directly; no finish event to wait for
    await call("utilities/uninstall_plugin", [name]);
  } else {
    throw new Error(`unknown command: ${cmd}`);
  }

  if (cmd === "install") await done; // wait for plugin_download_finish
  ws.close();
}

main().then(
  () => process.exit(0),
  (e) => {
    console.error(`decky-api: ${e.message}`);
    process.exit(1);
  },
);
