import { expect, type Locator, type Page } from "@playwright/test";

/** The modifier the app listens for: Meta on macOS, Control elsewhere. */
export const MOD = process.platform === "darwin" ? "Meta" : "Control";

export async function openApp(page: Page): Promise<void> {
  await page.goto("/?fast");
  await expect(page.getByRole("heading", { name: "aispice" })).toBeVisible();
}

/** From the welcome screen into the mock project, with rc_lowpass.asc open. */
export async function openProject(page: Page): Promise<void> {
  await openApp(page);
  await page.getByRole("button", { name: /Open folder/ }).click();
  await expect(page.getByRole("navigation", { name: "Circuits" })).toBeVisible();
  await expect(page.locator(".sch-host svg")).toBeVisible();
}

export function schematic(page: Page): Locator {
  return page.getByTestId("schematic-pane").first();
}

export async function zoomLevel(page: Page): Promise<number> {
  const text = await page.getByTestId("zoom-level").first().textContent();
  return Number(text?.replace("%", ""));
}

export async function simulate(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Simulate", exact: true }).click();
  await expect(page.locator(".plot canvas").first()).toBeVisible();
}

export async function sendChat(page: Page, text: string): Promise<void> {
  const box = page.getByLabel("Message");
  await box.fill(text);
  await box.press("Enter");
}

/** Clicks a point on the first plot, as fractions of its box. */
export async function clickPlot(page: Page, fx: number, fy: number, index = 0): Promise<void> {
  const box = await page.locator(".plot").nth(index).boundingBox();
  if (!box) throw new Error("no plot");
  await page.mouse.click(box.x + box.width * fx, box.y + box.height * fy);
}
