import { expect, test } from "@playwright/test";
import { clickPlot, MOD, openApp, openProject, schematic, sendChat, simulate, zoomLevel } from "./helpers";

/*
 * The built app on the in-browser mock backend, driven the way a person would.
 * Behaviour below the surface (reducers, the mock's numbers, sanitising) is
 * covered by the unit tests; these check that the pieces meet in a browser.
 */

test.describe("start", () => {
  test("the welcome screen checks the environment and opens a project", async ({ page }) => {
    await openApp(page);
    const env = page.getByRole("region", { name: "Environment" });
    await expect(env).toContainText("ngspice");
    await expect(env).toContainText("44.2");
    await expect(env).toContainText("Optional. Install Xyce");
    await expect(env).toContainText("Add a key in Settings.");
    await expect(page.getByRole("region", { name: "Recent projects" })).toContainText("sensor-frontend");

    await page.getByRole("button", { name: /Open folder/ }).click();
    const list = page.getByRole("navigation", { name: "Circuits" });
    await expect(list.getByRole("button", { name: /rc_lowpass.asc/ })).toHaveAttribute("aria-current", "true");
    await expect(list.getByRole("button")).toContainText(["ce_amp.asc", "opamp_inverting.asc", "rc_lowpass.asc"]);
    await expect(page.locator('.sch-host [data-inst="C1"]')).toBeVisible();
    await expect(page.locator(".statusbar")).toContainText("Auto (ngspice 44.2)");
  });

  test("Projects returns to the start screen", async ({ page }) => {
    await openProject(page);
    await page.getByRole("button", { name: "Projects" }).click();
    await expect(page.getByRole("heading", { name: "aispice" })).toBeVisible();
  });
});

test.describe("schematic", () => {
  test("opening another circuit shows its drawing and netlist", async ({ page }) => {
    await openProject(page);
    await page.getByRole("button", { name: /ce_amp.asc/ }).click();
    await expect(page.locator('.sch-host [data-inst="Q1"]')).toBeVisible();
    await page.getByRole("tab", { name: "Netlist" }).click();
    await expect(page.locator(".textview-pre")).toContainText("Q1 N002 N001 N003 2N3904");
    await expect(page.locator(".textview-pre")).toContainText(".step param RG list 100 220 470");
  });

  test("pans, zooms and fits with keys, wheel and drag", async ({ page }) => {
    await openProject(page);
    const pane = schematic(page);
    const svg = page.locator(".sch-host svg");
    const fitted = await zoomLevel(page);
    const box0 = await svg.getAttribute("viewBox");

    await pane.focus();
    await page.keyboard.press("+");
    expect(await zoomLevel(page)).toBeGreaterThan(fitted);
    await page.keyboard.press("ArrowRight");
    expect(await svg.getAttribute("viewBox")).not.toBe(box0);
    await page.keyboard.press("f");
    expect(await zoomLevel(page)).toBe(fitted);
    expect(await svg.getAttribute("viewBox")).toBe(box0);

    const b = (await pane.boundingBox())!;
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
    await page.mouse.wheel(0, -400);
    await expect.poll(() => zoomLevel(page)).toBeGreaterThan(fitted);

    await page.getByRole("button", { name: "Fit" }).click();
    const before = await svg.getAttribute("viewBox");
    await page.mouse.move(b.x + 40, b.y + 40);
    await page.mouse.down();
    await page.mouse.move(b.x + 140, b.y + 90, { steps: 5 });
    await page.mouse.up();
    expect(await svg.getAttribute("viewBox")).not.toBe(before);
    await page.getByRole("button", { name: "Zoom out" }).click();
    expect(await zoomLevel(page)).toBeLessThan(fitted);
  });

  test("clicking a part shows its value, pins and nets", async ({ page }) => {
    await openProject(page);
    await page.locator('.sch-host [data-inst="C1"] .sch-hit').click();
    const pop = page.getByRole("dialog", { name: "C1 details" });
    await expect(pop).toBeVisible();
    await expect(pop).toContainText("Capacitor");
    await expect(pop).toContainText("100n");
    await expect(pop.getByRole("row")).toContainText(["Pin", "A", "B"]);
    await expect(pop).toContainText("out");
    await page.keyboard.press("Escape");
    await expect(pop).toBeHidden();
  });

  test("a finding locates its part", async ({ page }) => {
    await openProject(page);
    await page.getByRole("button", { name: /opamp_inverting.asc/ }).click();
    const finding = page.getByRole("button", { name: /Net N004 reaches only one pin/ });
    await expect(finding).toBeVisible();
    await finding.click();
    await expect(page.getByRole("dialog", { name: "RL details" })).toContainText("N004");
    await expect(page.locator('.sch-host [data-inst="RL"][data-selected]')).toHaveCount(1);
  });
});

test.describe("simulation", () => {
  test("simulates and explores a Bode plot with cursors", async ({ page }) => {
    await openProject(page);
    await simulate(page);
    await expect(page.getByRole("tab", { name: "Waveforms" })).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(".plot .uplot")).toHaveCount(2); // magnitude and phase
    await expect(page.locator(".meas-list").first()).toContainText("1.592 kHz");

    const vin = page.getByRole("button", { name: "V(in)" });
    await expect(vin).toHaveAttribute("aria-pressed", "false");
    await vin.click();
    await expect(vin).toHaveAttribute("aria-pressed", "true");

    await clickPlot(page, 0.4, 0.5);
    await clickPlot(page, 0.75, 0.5);
    const readout = page.getByTestId("cursor-readout");
    await expect(readout).toContainText("frequency");
    await expect(readout).toContainText("V(out) mag");
    await expect(readout).toContainText("V(in) phase");
    await expect(readout.locator("td").first()).toContainText("Hz");

    await page.getByRole("tab", { name: "Log" }).click();
    await expect(page.getByLabel("Simulator log")).toContainText("ngspice-44.2");
  });

  test("overlays and filters the steps of a stepped run", async ({ page }) => {
    await openProject(page);
    await page.getByRole("button", { name: /ce_amp.asc/ }).click();
    await expect(page.locator('.sch-host [data-inst="Q1"]')).toBeVisible();
    await simulate(page);
    const step = page.getByLabel("Step");
    await expect(step).toHaveValue("all");
    await clickPlot(page, 0.5, 0.5);
    await expect(page.getByTestId("cursor-readout")).toContainText("V(out) RG=220");
    await step.selectOption("RG=470");
    await clickPlot(page, 0.3, 0.5);
    await expect(page.getByTestId("cursor-readout")).not.toContainText("RG=220");
    await expect(page.locator(".meas-list").first()).toContainText("gain RG=470");
  });

  test("a failed simulator says so and points at the log", async ({ page }) => {
    await openProject(page);
    await page.getByLabel("Simulator", { exact: true }).selectOption("xyce");
    await page.getByRole("button", { name: "Simulate", exact: true }).click();
    await expect(page.getByRole("alert").filter({ hasText: "Simulation failed" })).toBeVisible();
    await page.getByRole("tab", { name: "Waveforms" }).click();
    await expect(page.getByText("The simulation failed")).toBeVisible();
  });
});

test.describe("chat", () => {
  test("the scripted agent edits, simulates and checks specs, and the edit can be undone", async ({ page }) => {
    await openProject(page);
    await sendChat(page, "Move the corner to 1 kHz. The ADC input is about 100k.");

    const edit = page.locator(".tool-card[data-kind='edit']");
    await expect(edit.getByRole("button", { name: /Edited rc_lowpass.asc/ })).toHaveAttribute("aria-expanded", "true");
    await expect(edit.locator(".diff-line[data-kind='add']", { hasText: "SYMATTR Value 160n" })).toBeVisible();
    await expect(edit.locator(".diff-line[data-kind='del']", { hasText: "SYMATTR Value 100n" })).toBeVisible();
    await expect(edit).toContainText("Added R2 (res, 100k)");

    await expect(page.locator('.sch-host [data-inst="R2"][data-hl="added"]')).toHaveCount(1);
    await expect(page.locator('.sch-host [data-inst="C1"][data-hl="changed"]')).toHaveCount(1);
    await expect(page.locator(".sch-hl-box")).toHaveCount(2);
    await expect(page.locator(".sch-legend")).toContainText("1 added");

    await expect(page.getByRole("button", { name: "Simulated with ngspice in 0.4 s" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Checked 3 specs: 2 pass, 1 fails" })).toBeVisible();
    await expect(page.locator(".message[data-role='assistant'] .md table").last()).toContainText("Attenuation at 5 kHz");
    await page.getByRole("button", { name: /Thinking/ }).click();
    await expect(page.locator(".thinking-body")).toContainText("1/(2 pi R C)");

    await edit.getByRole("button", { name: "Undo" }).click();
    await expect(edit.getByRole("button", { name: "Undone" })).toBeDisabled();
    await page.getByRole("tab", { name: "Schematic" }).click();
    await expect(page.locator('.sch-host [data-inst="R2"]')).toHaveCount(0);
    await expect(page.locator(".sch-hl-box")).toHaveCount(0);
    await expect(page.getByLabel("Session")).toContainText("Move the corner to 1 kHz");
  });

  test("ask-before-apply shows the proposed diff and waits", async ({ page }) => {
    await openProject(page);
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Ask before applying" }).click();
    await page.getByRole("button", { name: "Save" }).click();
    await sendChat(page, "Move the corner to 1 kHz");
    const prompt = page.getByRole("alertdialog", { name: "Edit waiting for approval" });
    await expect(prompt).toContainText("Set C1 to 160n");
    await expect(page.locator('.sch-host [data-inst="R2"]')).toHaveCount(0);
    await prompt.getByRole("button", { name: "Apply" }).click();
    await expect(page.locator(".tool-card[data-kind='edit']")).toBeVisible();
    await expect(page.locator('.sch-host [data-inst="R2"]')).toHaveCount(1);
  });

  test("the model picker lists the provider's models", async ({ page }) => {
    await openProject(page);
    await page.getByRole("button", { name: /Anthropic claude-opus-5-5/ }).click();
    const menu = page.getByRole("dialog", { name: "Choose a model" });
    await expect(menu.getByRole("listbox", { name: "Models" }).getByRole("option")).toHaveCount(3);
    await menu.getByRole("button", { name: /Claude Haiku 5.5/ }).click();
    await expect(page.locator(".statusbar")).toContainText("claude-haiku-5-5");
  });
});

test.describe("specs and settings", () => {
  test("checks specs and shows pass and fail with margins", async ({ page }) => {
    await openProject(page);
    await page.getByRole("tab", { name: "Specs" }).click();
    await expect(page.getByText("Not checked yet")).toBeVisible();
    await page.getByRole("button", { name: "Check specs" }).click();
    const table = page.locator(".spec-table");
    await expect(table.getByRole("row")).toHaveCount(4);
    await expect(table).toContainText("Corner frequency");
    await expect(table.locator(".chip[data-tone='fail']")).toHaveCount(2);
    await expect(table.locator(".chip[data-tone='pass']")).toHaveCount(1);
    await expect(table.locator(".spec-margin[data-negative]").first()).toContainText("-");
  });

  test("history restores a snapshot", async ({ page }) => {
    await openProject(page);
    await page.getByRole("tab", { name: "History" }).click();
    await expect(page.locator(".history-row")).toHaveCount(2);
    await page.getByRole("button", { name: "Restore" }).click();
    await expect(page.locator(".history-row[data-current]")).toContainText("Opened rc_lowpass.asc");
    await page.getByRole("tab", { name: "Netlist" }).click();
    await expect(page.locator(".textview-pre")).toContainText("R1 in out 2.2k");
  });

  test("sets and removes a key without showing it, and saves settings", async ({ page }) => {
    await openApp(page);
    await page.getByRole("button", { name: "Open settings" }).click();
    const dialog = page.getByRole("dialog", { name: "Settings" });
    const openai = dialog.getByRole("listitem", { name: "OpenAI" });
    await openai.getByRole("button", { name: "Set key" }).click();
    await openai.getByLabel("OpenAI API key").fill("sk-e2e-0123456789");
    await openai.getByRole("button", { name: "Save key" }).click();
    await expect(openai.getByTestId("key-status-openai")).toHaveText("Key in system keychain");
    expect(await page.content()).not.toContain("sk-e2e-0123456789");
    await openai.getByRole("button", { name: "Remove" }).click();
    await expect(openai.getByTestId("key-status-openai")).toHaveText("No key");

    await dialog.getByRole("button", { name: "LTspice" }).click();
    await dialog.getByLabel("LTspice path").fill("/opt/ltspice/LTspice.exe");
    await dialog.getByRole("button", { name: "Save" }).click();
    await expect(page.getByRole("status").filter({ hasText: "Settings saved." })).toBeVisible();
    await page.getByRole("button", { name: "Open settings" }).click();
    await expect(dialog.getByRole("button", { name: "LTspice", exact: true })).toHaveAttribute("aria-pressed", "true");
    await expect(dialog.getByLabel("LTspice path")).toHaveValue("/opt/ltspice/LTspice.exe");
  });
});

test.describe("keyboard", () => {
  test("shortcuts open, simulate, undo, redo, switch tabs and send", async ({ page }) => {
    await openApp(page);
    await page.keyboard.press(`${MOD}+o`);
    await expect(page.locator(".sch-host svg")).toBeVisible();

    await page.keyboard.press(`${MOD}+r`);
    await expect(page.locator(".statusbar")).toContainText("Last run");
    await expect(page.getByRole("tab", { name: "Waveforms" })).toHaveAttribute("aria-selected", "true");

    await page.keyboard.press(`${MOD}+3`);
    await expect(page.getByRole("tab", { name: "Netlist" })).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press(`${MOD}+z`);
    await expect(page.locator(".textview-pre")).toContainText("R1 in out 2.2k");
    await page.keyboard.press(`${MOD}+Shift+z`);
    await expect(page.locator(".textview-pre")).toContainText("R1 in out 1k");

    await page.keyboard.press(`${MOD}+,`);
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog", { name: "Settings" })).toBeHidden();

    await page.keyboard.press("?");
    await expect(page.getByRole("dialog", { name: "Keyboard shortcuts" })).toContainText("Fit to view");
    await page.keyboard.press("Escape");

    await page.getByLabel("Message").fill("Explain how this circuit works");
    await page.getByRole("tab", { name: "Netlist" }).focus();
    await page.keyboard.press(`${MOD}+Enter`);
    await expect(page.locator(".message[data-role='user']")).toContainText("Explain how this circuit works");
    await expect(page.locator(".message[data-role='assistant']")).toContainText("first-order RC low-pass");
  });

  test("Undo stands aside while typing in a field", async ({ page }) => {
    await openProject(page);
    const box = page.getByLabel("Message");
    await box.fill("abc");
    await box.press(`${MOD}+z`);
    await page.getByRole("tab", { name: "Netlist" }).click();
    await expect(page.locator(".textview-pre")).toContainText("R1 in out 1k");
  });

  test("tabs move with arrow keys and splitters resize from the keyboard and persist", async ({ page }) => {
    await openProject(page);
    await page.getByRole("tab", { name: "Schematic" }).focus();
    await page.keyboard.press("ArrowRight");
    await expect(page.getByRole("tab", { name: "Waveforms" })).toBeFocused();
    await expect(page.getByRole("tab", { name: "Waveforms" })).toHaveAttribute("aria-selected", "true");

    const chat = page.getByRole("complementary", { name: "Chat" });
    const before = (await chat.boundingBox())!.width;
    const handle = page.getByRole("separator", { name: "Resize chat" });
    await handle.focus();
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("ArrowLeft");
    await expect.poll(async () => (await chat.boundingBox())!.width).toBe(before + 32);
    await page.reload();
    await page.getByRole("button", { name: /Open folder/ }).click();
    await expect.poll(async () => (await chat.boundingBox())!.width).toBe(before + 32);
  });
});

test.describe("themes and sizes", () => {
  test("follows the system theme and an explicit choice that survives a reload", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "dark" });
    await openApp(page);
    // Brightness of the page ground, so the check survives a change of palette.
    const bg = () =>
      page.evaluate(() => {
        const [r, g, b] = getComputedStyle(document.body).backgroundColor.match(/\d+/g)!.map(Number);
        return r + g + b;
      });
    expect(await bg()).toBeLessThan(150);
    await page.getByRole("button", { name: "Open settings" }).click();
    await page.getByRole("button", { name: "Light" }).click();
    await page.getByRole("button", { name: "Save" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    expect(await bg()).toBeGreaterThan(650);
    await page.reload();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  });

  test("a 1024 px window keeps all three panes usable without horizontal scroll", async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 700 });
    await openProject(page);
    const width = (sel: string) => page.locator(sel).first().evaluate((el) => el.getBoundingClientRect().width);
    expect(await width(".sidebar")).toBeGreaterThanOrEqual(180);
    expect(await width(".center")).toBeGreaterThanOrEqual(380);
    expect(await width(".chat")).toBeGreaterThanOrEqual(300);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow).toBeLessThanOrEqual(0);
    await expect(page.getByRole("button", { name: "Open in LTspice" })).toBeVisible();
    await simulate(page);
    await expect(page.getByTestId("cursor-readout").or(page.getByText("Click the plot to place cursor A"))).toBeVisible();
  });
});

/** Was a bug: under LANG=C the webview reports the language "C", uPlot's
 *  Intl.NumberFormat threw at load and the window stayed empty. */
test("starts when the system language is the C locale", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(Navigator.prototype, "language", { configurable: true, get: () => "C" });
    Object.defineProperty(Navigator.prototype, "languages", { configurable: true, get: () => ["C"] });
  });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1 })).toContainText("aispice");
  expect(errors).toEqual([]);
});
