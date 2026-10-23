/**
 * The screenshots, one entry each: the file written to docs/assets/<file>.png,
 * the route to open, and what to do there before the shot.
 */
import type { Page } from "playwright";

export interface Scene {
  file: string;
  route: string;
  prepare?: (page: Page) => Promise<void>;
}

export const SCENES: Scene[] = [
  { file: "wallet", route: "/" },
  { file: "auctions", route: "/auctions" },
  { file: "market", route: "/market" },
  {
    file: "market-listings",
    route: "/market",
    prepare: async (page) => {
      await page.getByRole("tab", { name: "My listings" }).click();
    },
  },
  {
    file: "namebase-migration",
    route: "/migration",
    prepare: async (page) => {
      await page.getByRole("button", { name: "Show details" }).click();
      // The shutdown notice above takes half the window; bring the history up.
      const card = page.locator("div.rounded", {
        has: page.getByRole("heading", { name: "Account history (imported)" }),
      });
      await card.last().evaluate((el) => el.scrollIntoView({ block: "start" }));
    },
  },
];
