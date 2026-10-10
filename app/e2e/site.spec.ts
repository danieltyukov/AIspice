import { expect, test, type Locator, type Page } from "@playwright/test";
import { clickPlot, openProject, sendChat } from "./helpers";

/*
 * Product renders for the project site, taken from the real interface on the
 * mock backend at twice the pixel density. They are written to
 * site/renders-src/ as PNG; site/tools/encode_renders.py turns them into the
 * WebP and AVIF files the page loads. Skipped unless SITE_RENDERS=1, so the
 * ordinary test run never writes into the site.
 *
 *   SITE_RENDERS=1 npx playwright test e2e/site.spec.ts
 */

test.skip(!process.env.SITE_RENDERS, "set SITE_RENDERS=1 to write the site renders");
test.use({ deviceScaleFactor: 2 });

const OUT = "../site/renders-src";
const SCHEMES = ["light", "dark"] as const;

async function settle(page: Page) {
  await page.evaluate(() => document.fonts.ready);
  await page.mouse.move(2, 450);
  await page.waitForTimeout(500);
}

async function shot(target: Locator, name: string) {
  await target.scrollIntoViewIfNeeded();
  await target.screenshot({ path: `${OUT}/${name}.png`, animations: "disabled" });
}

/** The edit turn: schematic with the change outlined, Bode plot with cursors, the edit card. */
async function editTurn(page: Page) {
  await openProject(page);
  await page.getByRole("button", { name: "Split" }).click();
  await sendChat(page, "Move the corner to 1 kHz. The ADC input is about 100k.");
  await expect(page.getByRole("button", { name: /Checked 3 specs/ })).toBeVisible();
  await expect(page.locator(".plot .uplot")).toHaveCount(2);
  await expect(page.locator(".sch-hl-box")).toHaveCount(2);
  await clickPlot(page, 0.475, 0.5);
  await clickPlot(page, 0.665, 0.5);
}

for (const scheme of SCHEMES) {
  test(`site renders ${scheme}`, async ({ page }) => {
    test.slow();
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ colorScheme: scheme });

    await editTurn(page);
    await page.locator(".tool-card[data-kind='edit']").evaluate((el) => el.scrollIntoView({ block: "start" }));
    await page.locator(".chat-scroll").evaluate((el) => el.scrollBy(0, -52));
    await settle(page);
    await page.screenshot({ path: `${OUT}/hero-${scheme}.png` });

    // The pieces, each on its own.
    await shot(page.locator(".sch-host").first(), `schematic-${scheme}`);
    await shot(page.locator(".tool-card[data-kind='edit']"), `diff-${scheme}`);
    await shot(page.locator(".wave-body").first(), `bode-${scheme}`);
    const specCard = page.locator(".tool-card[data-kind='specs']");
    if (!(await specCard.getAttribute("data-open"))) {
      const head = specCard.locator(".tool-head");
      if ((await head.getAttribute("aria-expanded")) === "false") await head.click();
    }
    await settle(page);
    await shot(specCard, `specs-${scheme}`);

    // Sizing and yield, in a new chat on the same circuit.
    await page.getByRole("button", { name: "New", exact: true }).click();
    await sendChat(page, "Optimize R1 and C1 for the specs, then check the yield with real tolerances.");
    await expect(page.getByRole("button", { name: /Monte Carlo/ })).toBeVisible();
    for (const kind of ["optimize", "montecarlo"]) {
      const head = page.locator(`.tool-card[data-kind='${kind}'] .tool-head`);
      if ((await head.getAttribute("aria-expanded")) === "false") await head.click();
    }
    await settle(page);
    await shot(page.locator(".tool-card[data-kind='optimize']"), `optimize-${scheme}`);
    await shot(page.locator(".tool-card[data-kind='montecarlo']"), `montecarlo-${scheme}`);
  });
}
