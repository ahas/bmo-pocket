// The pocket screen's view switch (docs/mcp-host.md, 「How the screen reads」), driven in Chromium at a wide window
// and at a phone's width, every list the screen reads answered empty: the switch is bmo-ui's one select, whose four
// views open on no search, centred in the screen's row, and the view chosen there is the one the screen's address
// names. Whether a list searches is the layer's own rule and its own tests; this holds the screen's list to it.

import { afterAll, beforeAll, expect, test } from "bun:test";

import words from "../../i18n/locales/ko.json";
import { standScreen } from "../support/screen";

import type { Screen } from "../support/screen";

let screen: Screen;

// What a test may take at most, a bound and never a wait: the dev server builds the screen on its first page.
const STANDING_MS = 300_000;
const DRIVING_MS = 120_000;

beforeAll(async () => {
  screen = await standScreen();
}, STANDING_MS);

afterAll(async () => {
  await screen?.close();
});

for (const width of [
  { name: "a wide window", viewport: { width: 1280, height: 900 }, touch: false },
  { name: "a phone", viewport: { width: 360, height: 780 }, touch: true },
]) {
  test(`at ${width.name}, the view switch opens its four views on no search and the view chosen is the screen's`, async () => {
    const context = await screen.browser.newContext({ viewport: width.viewport, locale: "ko-KR", hasTouch: width.touch, isMobile: width.touch });

    try {
      // Every list the screen reads through the relay, answered as a host holding none.
      await context.route(
        (url) => url.pathname.startsWith("/pocket/"),
        (route) => (route.request().resourceType() === "fetch" ? route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ items: [], more: false }) }) : route.continue()),
      );

      const page = await context.newPage();

      await page.goto(`${screen.origin}/pocket`);

      const select = page.locator(`button[aria-label="${words.pocket.label.view}"]`).first();

      await select.waitFor();

      // The switch stands centred in its row, which stays on the screen's width.
      const placed = await select.evaluate((element) => {
        const own = element.getBoundingClientRect();
        const row = element.closest("[role=toolbar]")!.getBoundingClientRect();

        return { above: Math.round(own.top - row.top), below: Math.round(row.bottom - own.bottom), wide: document.documentElement.scrollWidth };
      });

      expect(placed.above).toBe(placed.below);
      expect(placed.wide).toBe(width.viewport.width);

      await select.click();

      const list = page.locator('[data-slot="content"][data-side]:has([role="listbox"])');

      await list.waitFor();
      expect([await list.getByRole("option").count(), await list.locator("input").count()]).toEqual([4, 0]);
      await list.getByRole("option", { name: words.pocket.view.schedules, exact: true }).click();
      await page.waitForURL((url) => url.searchParams.get("view") === "schedules");
    } finally {
      await context.close();
    }
  }, DRIVING_MS);
}
