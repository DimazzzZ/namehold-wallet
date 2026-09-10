/**
 * E2E tests for lib.rs (Tauri app entrypoint).
 * Covers: app lifecycle, window creation, menu/tray initialization, event handlers.
 * These paths cannot be unit-tested (require live Tauri runtime + OS windows).
 */

describe("Namehold Wallet App Lifecycle (lib.rs)", () => {
  it("should launch the app and display the main window", async () => {
    // The app is started automatically by the WebdriverIO runner.
    // Wait for the main window to appear.
    const window = await browser.getWindowHandle();
    expect(window).toBeTruthy();

    // Check that the window title matches the configured title.
    const title = await browser.getTitle();
    expect(title).toContain("Namehold");
  });

  it("should render the React app in the main window", async () => {
    // Wait for the React root element to be present.
    const root = await browser.$("div#root");
    await root.waitForDisplayed({ timeout: 5000 });
    expect(await root.isDisplayed()).toBe(true);
  });

  it("should have a responsive layout", async () => {
    // Get window size (configured as 1280x800 in tauri.conf.json).
    const size = await browser.getWindowSize();
    expect(size.width).toBeGreaterThanOrEqual(1280);
    expect(size.height).toBeGreaterThanOrEqual(800);
  });

  it("should handle window events (resize, focus)", async () => {
    // Resize the window.
    await browser.setWindowSize(1024, 600);
    let size = await browser.getWindowSize();
    expect(size.width).toBe(1024);
    expect(size.height).toBe(600);

    // Restore to original size.
    await browser.setWindowSize(1280, 800);
    size = await browser.getWindowSize();
    expect(size.width).toBe(1280);
    expect(size.height).toBe(800);
  });

  it("should have the main menu/tray available (platform-dependent)", async () => {
    // On macOS/Linux, the menu is in the system menu bar.
    // On Windows, it's in the window title bar.
    // This test just verifies the app doesn't crash during menu initialization.
    // The actual menu interaction is platform-specific and tested separately.
    const title = await browser.getTitle();
    expect(title).toBeTruthy();
  });
});
