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
  { file: "names", route: "/names" },
  // A name of ours that is also an applied-for string in ICANN's 2026 round:
  // the ICANN badge in the header and the row in Details.
  { file: "name", route: "/names/atlas" },
  { file: "market", route: "/market" },
  {
    file: "market-listings",
    route: "/market",
    prepare: async (page) => {
      await page.getByRole("tab", { name: "My listings" }).click();
    },
  },
  // The exchange: the order book over Nostr, then the trades, one of them
  // waiting for our confirmation.
  { file: "exchange", route: "/exchange" },
  {
    file: "exchange-trades",
    route: "/exchange",
    prepare: async (page) => {
      await page.getByRole("tab", { name: /^My trades/ }).click();
      // Off the tab, so its tooltip does not cover the page.
      await page.mouse.move(0, 0);
    },
  },
  { file: "activity", route: "/activity" },
  { file: "settings", route: "/settings" },
];
