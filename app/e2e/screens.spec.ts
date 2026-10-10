import { expect, test, type Page } from "@playwright/test";
import { clickPlot, openApp, openProject, sendChat, simulate } from "./helpers";

/*
 * Pictures of every main screen, in both themes at a large and a small
 * window, kept for looking at rather than compared pixel by pixel (that would
 * test font rasterisation, not the app). The two hero images are the ones the
 * project site uses.
 */

const SCHEMES = ["light", "dark"] as const;
const SIZES = [
  { width: 1440, height: 900 },
  { width: 1024, height: 700 },
] as const;

async function settle(page: Page) {
  // Fonts, a dialog's fade and uPlot's first paint.
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(450);
}

async function shoot(page: Page, name: string, tag: string) {
  await settle(page);
  await page.screenshot({ path: `e2e/screenshots/review/${name}-${tag}.png` });
}

for (const scheme of SCHEMES) {
  for (const size of SIZES) {
    const tag = `${scheme}-${size.width}`;
    test(`screens ${tag}`, async ({ page }) => {
      test.slow();
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: scheme });

      await openApp(page);
      await expect(page.getByRole("region", { name: "Environment" })).toContainText("ngspice");
      await shoot(page, "01-welcome", tag);

      await page.getByRole("button", { name: /Open folder/ }).click();
      await expect(page.locator(".sch-host svg")).toBeVisible();
      await shoot(page, "02-schematic", tag);

      await page.locator('.sch-host [data-inst="C1"] .sch-hit').click();
      await expect(page.getByRole("dialog", { name: "C1 details" })).toBeVisible();
      await shoot(page, "03-part", tag);
      await page.keyboard.press("Escape");

      await simulate(page);
      await clickPlot(page, 0.42, 0.4);
      await clickPlot(page, 0.7, 0.4);
      await shoot(page, "04-bode", tag);

      await page.getByRole("button", { name: /ce_amp.asc/ }).click();
      await expect(page.getByRole("button", { name: /ce_amp.asc/ })).toHaveAttribute("aria-current", "true");
      await simulate(page);
      await clickPlot(page, 0.5, 0.4);
      await clickPlot(page, 0.66, 0.4);
      await shoot(page, "05-steps", tag);

      await page.getByRole("tab", { name: "Netlist" }).click();
      await shoot(page, "06-netlist", tag);
      await page.getByRole("tab", { name: "Log" }).click();
      await shoot(page, "07-log", tag);
      await page.getByRole("tab", { name: "Specs" }).click();
      await page.getByRole("button", { name: "Check specs" }).click();
      await expect(page.locator(".spec-table")).toBeVisible();
      await shoot(page, "08-specs", tag);
      await page.getByRole("tab", { name: "History" }).click();
      await shoot(page, "09-history", tag);

      await page.getByRole("button", { name: /opamp_inverting.asc/ }).click();
      await page.getByRole("tab", { name: "Schematic" }).click();
      await expect(page.locator(".findings")).toBeVisible();
      await shoot(page, "10-findings", tag);

      await page.getByRole("button", { name: /rc_lowpass.asc/ }).click();
      await expect(page.getByRole("button", { name: /rc_lowpass.asc/ })).toHaveAttribute("aria-current", "true");
      await simulate(page);
      await page.getByRole("button", { name: "Split" }).click();
      await sendChat(page, "Move the corner to 1 kHz. The ADC input is about 100k.");
      await expect(page.getByRole("button", { name: /Checked 3 specs/ })).toBeVisible();
      await shoot(page, "11-chat", tag);

      await page.getByRole("button", { name: "Settings", exact: true }).click();
      await expect(page.getByRole("dialog", { name: "Settings" })).toBeVisible();
      await shoot(page, "12-settings", tag);
      await page.keyboard.press("Escape");

      await page.getByRole("button", { name: "Keyboard shortcuts" }).click();
      await expect(page.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeVisible();
      await shoot(page, "13-shortcuts", tag);
    });
  }
}

/** The site's hero: schematic with edit highlights over a Bode plot, and the chat showing the edit card. */
async function hero(page: Page, scheme: "light" | "dark") {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ colorScheme: scheme });
  await openProject(page);
  await page.getByRole("button", { name: "Split" }).click();
  await sendChat(page, "Move the corner to 1 kHz. The ADC input is about 100k.");
  await expect(page.getByRole("button", { name: /Checked 3 specs/ })).toBeVisible();
  await expect(page.locator(".plot .uplot")).toHaveCount(2);
  await expect(page.locator(".sch-hl-box")).toHaveCount(2);
  // Cursor A on the corner, B a decade above.
  await clickPlot(page, 0.475, 0.5);
  await clickPlot(page, 0.665, 0.5);
  // Show the edit card from its title down.
  await page.locator(".tool-card[data-kind='edit']").evaluate((el) => el.scrollIntoView({ block: "start" }));
  await page.locator(".chat-scroll").evaluate((el) => el.scrollBy(0, -52));
  await page.mouse.move(2, 450);
  await settle(page);
  await page.screenshot({ path: `e2e/screenshots/hero-${scheme}.png` });
}

test("hero light", async ({ page }) => {
  await hero(page, "light");
});

test("hero dark", async ({ page }) => {
  await hero(page, "dark");
});
