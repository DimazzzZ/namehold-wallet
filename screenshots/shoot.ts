/**
 * Generates the README screenshots: starts the Vite dev server (the app then
 * runs on its browser mock), installs the showcase wallet over the mock, shoots
 * each scene and frames it as a macOS window into docs/assets/.
 *
 *   pnpm shoot            every scene
 *   pnpm shoot market     only the named scenes
 */
import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { chromium, type Page } from "playwright";
import { frameHtml } from "./frame";
import { SCENES } from "./scenes";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const OUT_DIR = path.join(ROOT, "docs/assets");
const PORT = 5199;
const BASE = `http://localhost:${PORT}`;
/** The window's content area, in CSS pixels; shot at 2x for Retina. */
const VIEWPORT = { width: 1400, height: 900 };
const SCALE = 2;
/**
 * A fixed moment, in a fixed locale and time zone: a release changes an image
 * only when its screen changed, not because the day or the machine did.
 */
const NOW = new Date("2026-10-23T12:00:00Z");
const LOCALE = { locale: "en-US", timezoneId: "UTC" };

async function main() {
  const wanted = process.argv.slice(2);
  const unknown = wanted.filter((w) => !SCENES.some((s) => s.file === w));
  if (unknown.length) throw new Error(`unknown scene(s): ${unknown.join(", ")}`);
  const scenes = wanted.length ? SCENES.filter((s) => wanted.includes(s.file)) : SCENES;

  const showcase = await bundleShowcase();
  const server = await startVite();
  const browser = await chromium.launch();
  // Written only once every scene is shot, so a failing scene leaves the old set whole.
  const shots: { out: string; png: Buffer }[] = [];
  try {
    for (const scene of scenes) {
      const page = await browser.newPage({
        viewport: VIEWPORT,
        deviceScaleFactor: SCALE,
        ...LOCALE,
      });
      await page.clock.setFixedTime(NOW);
      await page.addInitScript(showcase);
      const unmocked = watchUnmocked(page);
      await page.goto(BASE + scene.route);
      await settle(page);
      await scene.prepare?.(page);
      await settle(page);
      const shot = await page.screenshot({ type: "png" });
      await page.close();
      const out = path.join(OUT_DIR, `${scene.file}.png`);
      shots.push({ out, png: await frame(browser, shot) });
      const missing = [...unmocked].join(", ");
      console.log(
        `✓ ${path.relative(ROOT, out)}${missing ? `  (no mock answer: ${missing})` : ""}`,
      );
    }
  } finally {
    await browser.close();
    // Detached into its own process group: this ends vite, not only `pnpm exec`.
    process.kill(-server.pid!);
  }
  mkdirSync(OUT_DIR, { recursive: true });
  for (const { out, png } of shots) writeFileSync(out, png);
}

/** The showcase as one script the page runs before the app's own code. */
async function bundleShowcase(): Promise<string> {
  const result = await build({
    entryPoints: [path.join(HERE, "showcase/index.ts")],
    bundle: true,
    format: "iife",
    platform: "browser",
    write: false,
  });
  return result.outputFiles[0]!.text;
}

/** Starts Vite and waits until it answers, or fails after 30 s with its log. */
async function startVite(): Promise<ChildProcess> {
  const vite = spawn("pnpm", ["exec", "vite", "--port", String(PORT), "--strictPort"], {
    cwd: ROOT,
    stdio: ["ignore", "pipe", "pipe"],
    detached: true,
  });
  let log = "";
  let exited: number | null = null;
  vite.stdout!.on("data", (c: Buffer) => (log += c));
  vite.stderr!.on("data", (c: Buffer) => (log += c));
  vite.on("exit", (code) => (exited = code ?? -1));
  for (let waited = 0; waited < 30_000; waited += 250) {
    if (exited !== null) throw new Error(`vite exited (${exited}):\n${log}`);
    const up = await fetch(BASE).then(
      (r) => r.ok,
      () => false,
    );
    if (up) return vite;
    await new Promise((r) => setTimeout(r, 250));
  }
  process.kill(-vite.pid!);
  throw new Error(`vite did not answer on ${BASE} within 30 s:\n${log}`);
}

/** Commands the mock had no answer for, from its own console warning. */
function watchUnmocked(page: Page): Set<string> {
  const seen = new Set<string>();
  page.on("console", (msg) => {
    const m = /No mock handler for invoke\("([^"]+)"\)/.exec(msg.text());
    if (m) seen.add(m[1]!);
  });
  return seen;
}

/** Wait for the app's queries to land and its transitions to finish. */
async function settle(page: Page) {
  await page.waitForLoadState("networkidle");
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(800);
}

async function frame(browser: import("playwright").Browser, shot: Buffer): Promise<Buffer> {
  const page = await browser.newPage({ deviceScaleFactor: SCALE });
  await page.setContent(frameHtml(shot.toString("base64"), VIEWPORT));
  const png = await page.locator("#stage").screenshot({ omitBackground: true });
  await page.close();
  return png;
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
