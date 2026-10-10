// End-to-end test of the real desktop app: the Tauri binary with its Rust
// backend, driven over WebDriver (tauri-driver + WebKitWebDriver), against a
// real project folder and a real simulator. The language model is a small
// OpenAI-compatible server scripted below, so the whole path runs for real:
// UI, IPC, agent loop, HTTP provider, tools, simulator, files on disk.
//
//   xvfb-run -a node e2e-desktop/run.mjs        (from app/)
//
// No dependencies: WebDriver is spoken over fetch.

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";

const here = resolve(import.meta.dirname);
const binary = resolve(here, "../../target/debug/aispice-app");
const out = join(here, "output");
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
if (!existsSync(binary)) throw new Error(`build the app first: npx tauri build --debug --no-bundle (${binary})`);

// A project folder with one circuit and a home for aispice's settings.
const root = mkdtempSync(join(tmpdir(), "aispice-e2e-"));
const project = join(root, "filters");
const config = join(root, "config");
const data = join(root, "data");
mkdirSync(project);
mkdirSync(join(config, "aispice"), { recursive: true });
writeFileSync(
  join(project, "rc.asc"),
  [
    "Version 4",
    "SHEET 1 880 680",
    "WIRE 96 96 32 96",
    "WIRE 240 96 176 96",
    "WIRE 240 128 240 96",
    "FLAG 32 176 0",
    "FLAG 240 192 0",
    "FLAG 240 96 out",
    "SYMBOL voltage 32 80 R0",
    "SYMATTR InstName V1",
    "SYMATTR Value 0",
    "SYMATTR SpiceLine AC 1",
    "SYMBOL res 192 80 R90",
    "SYMATTR InstName R1",
    "SYMATTR Value 1k",
    "SYMBOL cap 224 128 R0",
    "SYMATTR InstName C1",
    "SYMATTR Value 100n",
    "TEXT 0 232 Left 2 !.ac dec 50 10 1Meg",
    "",
  ].join("\r\n"),
);
writeFileSync(join(project, "rc.specs"), "bw = bandwidth_3db(V(out)) in 1.5k..1.7k\n");

// The scripted model.
const turns = [
  { tool: "edit_schematic", args: { circuit: "rc.asc", edits: [{ op: "set_value", name: "R1", value: "2.2k" }], reason: "Lower the corner" } },
  { tool: "simulate", args: { circuit: "rc.asc", measurements: ["f3db = bandwidth_3db(V(out))"] } },
  { text: "R1 is now 2.2k, which puts the corner near 723 Hz." },
];
let call = 0;
const requests = [];
const llm = createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    if (req.url.endsWith("/models")) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ object: "list", data: [{ id: "mock-model", object: "model" }] }));
      return;
    }
    requests.push(JSON.parse(body || "{}"));
    const t = turns[Math.min(call++, turns.length - 1)];
    res.writeHead(200, { "content-type": "text/event-stream" });
    const send = (o) => res.write(`data: ${JSON.stringify(o)}\n\n`);
    if (t.tool) {
      send({ choices: [{ index: 0, delta: { role: "assistant", tool_calls: [{ index: 0, id: `call_${call}`, type: "function", function: { name: t.tool, arguments: JSON.stringify(t.args) } }] } }] });
      send({ choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }] });
    } else {
      send({ choices: [{ index: 0, delta: { role: "assistant", content: t.text } }] });
      send({ choices: [{ index: 0, delta: {}, finish_reason: "stop" }] });
    }
    res.end("data: [DONE]\n\n");
  });
});
await new Promise((r) => llm.listen(0, "127.0.0.1", r));
const llmUrl = `http://127.0.0.1:${llm.address().port}/v1`;
writeFileSync(
  join(config, "aispice", "config.toml"),
  `provider = "mock"\nmodel = "mock-model"\nsimulator = "ngspice"\nedit_mode = "apply"\n\n[agent]\nmax_steps = 10\nthinking = false\n\n[providers.mock]\nbase_url = "${llmUrl}"\n`,
);
writeFileSync(join(config, "aispice", "app.json"), JSON.stringify({ recent_projects: [project], reload_ltspice: false, theme: "light" }));

// tauri-driver, which starts the app through WebKitWebDriver.
const driverBin = existsSync(join(homedir(), ".cargo/bin/tauri-driver")) ? join(homedir(), ".cargo/bin/tauri-driver") : "tauri-driver";
const driver = spawn(driverBin, ["--port", "4444"], { stdio: ["ignore", "inherit", "inherit"], env: {
    ...process.env,
    XDG_CONFIG_HOME: config,
    XDG_DATA_HOME: data,
    // On a Wayland desktop GTK would ignore xvfb-run's DISPLAY and open the
    // app on the real screen; keep it on the virtual display.
    GDK_BACKEND: "x11",
    WAYLAND_DISPLAY: "",
  },
});
const W = "http://127.0.0.1:4444";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function wd(method, path, body) {
  const r = await fetch(W + path, { method, headers: { "content-type": "application/json" }, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(`${method} ${path}: ${JSON.stringify(j.value ?? j)}`);
  return j.value;
}
for (let i = 0; i < 50; i++) {
  try {
    await fetch(W + "/status");
    break;
  } catch {
    await sleep(200);
  }
}
const session = await wd("POST", "/session", { capabilities: { alwaysMatch: { "tauri:options": { application: binary } } } });
const sid = session.sessionId;
const EL = "element-6066-11e4-a52e-4f735466cecf";

async function find(css, timeout = 20000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      const v = await wd("POST", `/session/${sid}/element`, { using: "css selector", value: css });
      return v[EL];
    } catch (e) {
      last = e;
      await sleep(200);
    }
  }
  throw new Error(`not found: ${css} (${last})`);
}
// Rendered text, with runs of whitespace (including no-break spaces and the
// line breaks WebKit puts between block children) folded to one space.
const norm = (t) => String(t).replace(/[\s\u00a0]+/g, " ").trim();
async function findByText(css, text, timeout = 30000) {
  const end = Date.now() + timeout;
  let seen = [];
  while (Date.now() < end) {
    const els = await wd("POST", `/session/${sid}/elements`, { using: "css selector", value: css });
    seen = [];
    for (const e of els) {
      let t = norm(await wd("GET", `/session/${sid}/element/${e[EL]}/text`).catch(() => ""));
      // WebKitWebDriver reports some visible elements (inside the chat's
      // scroll container) as not displayed and returns no text for them.
      if (!t) t = norm(await js("return arguments[0].textContent", [e]).catch(() => ""));
      if (t.includes(text)) return e[EL];
      seen.push(t.slice(0, 160));
    }
    await sleep(250);
  }
  throw new Error(`no ${css} containing "${text}"; saw ${JSON.stringify(seen)}`);
}
const click = (el) => wd("POST", `/session/${sid}/element/${el}/click`, {});
const type = (el, text) => wd("POST", `/session/${sid}/element/${el}/value`, { text });
const js = (script, args = []) => wd("POST", `/session/${sid}/execute/sync`, { script, args });
async function shot(name) {
  const b64 = await wd("GET", `/session/${sid}/screenshot`);
  writeFileSync(join(out, `${name}.png`), Buffer.from(b64, "base64"));
}
const results = [];
async function step(name, fn) {
  const t0 = Date.now();
  try {
    await fn();
    results.push({ name, ok: true, ms: Date.now() - t0 });
    console.log(`ok   ${name} (${Date.now() - t0} ms)`);
  } catch (e) {
    results.push({ name, ok: false, error: String(e) });
    console.log(`FAIL ${name}: ${e}`);
    await shot(`fail-${name.replace(/\W+/g, "-")}`).catch(() => {});
    // What the page itself knows: load state, errors caught before the
    // interface started (public/boot.js), and the text on screen.
    const page = await js(
      "var r = document.getElementById('root'); return { url: location.href, ready: document.readyState, bootErrors: window.__aispiceBootErrors || null, text: r ? r.innerText.slice(0, 400) : null };",
    ).catch((err) => ({ unavailable: String(err) }));
    console.log(`     page: ${JSON.stringify(page)}`);
    throw e;
  }
}

try {
  await wd("POST", `/session/${sid}/window/rect`, { width: 1440, height: 900 }).catch(() => {});
  await step("welcome screen shows the real environment", async () => {
    await findByText("h1, h2, .welcome", "aispice");
    await findByText("section, .panel, [role=region]", "ngspice");
    await shot("1-welcome");
  });
  await step("open the project from the recent list", async () => {
    await click(await findByText(".recent-item", "filters"));
    await find('nav[aria-label="Circuits"] button');
    await find('.sch-host [data-inst="R1"]');
    await shot("2-project");
  });
  await step("simulate on ngspice and plot", async () => {
    await click(await findByText("button", "Simulate"));
    await find(".plot canvas", 60000);
    await findByText(".statusbar, .status, footer", "ngspice", 30000);
    await shot("3-simulated");
  });
  await step("check the saved specs", async () => {
    await click(await findByText('[role="tab"]', "Specs"));
    await click(await findByText("button", "Check specs"));
    await findByText(".spec-table, table, .specs", "bw", 60000);
    await shot("4-specs");
  });
  await step("ask the agent to change R1; it edits, simulates and answers", async () => {
    const box = await find('textarea[aria-label="Message"]');
    await type(box, "Change R1 to 2.2k and check the bandwidth");
    await findByText(".chat, [aria-label=Chat]", "723 Hz", 90000);
    const asc = readFileSync(join(project, "rc.asc"), "utf8");
    if (!asc.includes("SYMATTR Value 2.2k")) throw new Error("file was not edited:\n" + asc);
    await findByText(".tool-title", "Edited rc.asc", 10000);
    await shot("5-agent-edit");
  });
  await step("the schematic refreshed with the new value", async () => {
    await click(await findByText('[role="tab"]', "Schematic"));
    await findByText(".sch-host", "2.2k", 20000);
  });
  await step("undo from the edit card restores the file", async () => {
    const undo = await findByText(".chat button, [aria-label=Chat] button", "Undo", 10000);
    await click(undo);
    const end = Date.now() + 15000;
    while (Date.now() < end) {
      if (readFileSync(join(project, "rc.asc"), "utf8").includes("SYMATTR Value 1k")) break;
      await sleep(250);
    }
    if (!readFileSync(join(project, "rc.asc"), "utf8").includes("SYMATTR Value 1k")) throw new Error("undo did not restore 1k");
    await shot("6-undone");
  });
  await step("the conversation is saved as a session", async () => {
    // In the user's data directory, never inside the project, where a cloned
    // repository could plant a forged conversation.
    if (existsSync(join(project, ".aispice/sessions"))) throw new Error("sessions written into the project");
    const dirs = existsSync(join(data, "aispice/sessions")) ? readdirSync(join(data, "aispice/sessions")) : [];
    const files = dirs.flatMap((d) => readdirSync(join(data, "aispice/sessions", d)));
    if (!files.some((f) => f.endsWith(".json"))) throw new Error("no saved session");
  });
  await step("the model received our tools and the system prompt", async () => {
    const first = requests[0];
    const names = first.tools.map((t) => t.function.name);
    for (const n of ["edit_schematic", "simulate", "check_specs", "optimize", "monte_carlo"]) if (!names.includes(n)) throw new Error(`missing tool ${n}`);
    if (!first.messages[0].content.includes("aispice")) throw new Error("no system prompt");
  });
} finally {
  await wd("DELETE", `/session/${sid}`).catch(() => {});
  driver.kill();
  llm.close();
  writeFileSync(join(out, "results.json"), JSON.stringify(results, null, 2));
}
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length} of ${results.length} steps passed`);
process.exit(failed.length ? 1 : 0);
