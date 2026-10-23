/**
 * A macOS window drawn around a screenshot of the app's content: title bar with
 * the traffic lights and the app's native title (`tauri.conf.json`), rounded
 * corners and a drop shadow on a transparent stage.
 */
const TITLE = "Namehold - your HNS network wallet";
const BAR = 38;
/** Room around the window for its shadow. */
const PAD = 40;

export function frameHtml(pngBase64: string, size: { width: number; height: number }): string {
  return `<!doctype html>
<html><head><meta charset="utf-8"><style>
  html, body { margin: 0; background: transparent; }
  #stage { display: inline-block; padding: ${PAD}px; }
  .window {
    width: ${size.width}px; border-radius: 12px; overflow: hidden; background: #fff;
    box-shadow: 0 0 0 0.5px rgba(0,0,0,.22), 0 22px 48px rgba(0,0,0,.24), 0 6px 14px rgba(0,0,0,.10);
  }
  .bar {
    height: ${BAR}px; display: flex; align-items: center; gap: 8px; padding: 0 14px;
    background: #f6f6f6; border-bottom: 1px solid #e2e2e2;
    font: 600 13px -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif; color: #3c3c3c;
  }
  .dot { width: 12px; height: 12px; border-radius: 50%; box-shadow: inset 0 0 0 0.5px rgba(0,0,0,.18); }
  .title { margin-left: 10px; }
  img { display: block; width: ${size.width}px; height: ${size.height}px; }
</style></head><body>
  <div id="stage"><div class="window">
    <div class="bar">
      <span class="dot" style="background:#ff5f57"></span>
      <span class="dot" style="background:#febc2e"></span>
      <span class="dot" style="background:#28c840"></span>
      <span class="title">${TITLE}</span>
    </div>
    <img src="data:image/png;base64,${pngBase64}">
  </div></div>
</body></html>`;
}
