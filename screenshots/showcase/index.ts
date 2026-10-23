/**
 * Bundled into the page before the app starts (see shoot.ts): answers the
 * browser mock's reads from the showcase wallet through the mock's
 * `__webqa_overrides` hook. Everything not listed here keeps the mock's own
 * answer (the market's listings among them).
 */
import * as world from "./world";

const now = new Date();

globalThis.__webqa_overrides = {
  list_wallet_profiles: world.profiles(now),
  get_signer_session: {
    walletProfileId: world.profiles(now)[0]!.id,
    unlocked: true,
    unlockedUntilEpochMs: now.getTime() + 3_600_000,
  },
  list_receive_addresses: world.receiveAddresses(now),
  get_wallet_balances: world.balances(),
  read_balance: world.legacyBalance(),
  read_names: world.names(),
  read_renewals: world.renewals(),
  read_auction_position_names: world.auctionPositionNames(),
  get_name_action_capabilities: (args?: Record<string, unknown>) =>
    world.capabilities(String(args?.name)),
  get_names_action_capabilities: (args?: Record<string, unknown>) =>
    ((args?.names as string[]) ?? []).map(world.capabilities),
  shakedex_list_listings: world.myListings(now),
  get_namebase_history_summary: world.namebaseSummary(),
  get_namebase_history: world.namebaseHistory(),
};
