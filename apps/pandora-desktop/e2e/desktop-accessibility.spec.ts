import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page, type TestInfo } from "@playwright/test";

const viewports = [
  { label: "100% scale", width: 1080, height: 720 },
  { label: "150% scale", width: 720, height: 480 },
  { label: "200% scale", width: 540, height: 360 },
];

const layouts = [
  { label: "right dock", open: true, placement: "right" },
  { label: "bottom dock", open: true, placement: "bottom" },
  { label: "hidden dock", open: false, placement: "right" },
] as const;

async function retainScreenshot(page: Page, testInfo: TestInfo, label: string) {
  const filename = `${label.replace(/[^a-z0-9]+/gi, "-").replace(/^-|-$/g, "").toLowerCase()}.png`;
  const path = testInfo.outputPath(filename);
  await page.screenshot({ path, fullPage: true });
  await testInfo.attach(label, { path, contentType: "image/png" });
}

test("Command Center has no automated accessibility violations", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1080, height: 720 });
  await page.goto("/");

  await expect(page.getByRole("main")).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Pandora navigation" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Search", exact: true })).toBeVisible();

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
  await retainScreenshot(page, testInfo, "command-center-baseline");
});

test("keyboard-only navigation exposes the skip target and visible focus", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1080, height: 720 });
  await page.goto("/");

  await page.keyboard.press("Tab");
  const skipLink = page.getByRole("link", { name: "Skip to workspace" });
  await expect(skipLink).toBeFocused();
  await expect(skipLink).toBeVisible();
  expect(await skipLink.evaluate((element) => getComputedStyle(element).outlineStyle)).not.toBe("none");
  await retainScreenshot(page, testInfo, "keyboard-skip-link-focus");

  await page.keyboard.press("Enter");
  await expect(page.getByRole("main")).toBeFocused();

  const focusTrail: string[] = [];
  for (let index = 0; index < 16; index += 1) {
    await page.keyboard.press("Tab");
    const focused = page.locator(":focus");
    await expect(focused).toBeVisible();
    const focusState = await focused.evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        identity: element.getAttribute("aria-label") ?? element.textContent?.trim() ?? element.tagName,
        outline: style.outlineStyle,
        shadow: style.boxShadow,
      };
    });
    expect(focusState.outline !== "none" || focusState.shadow !== "none").toBe(true);
    focusTrail.push(focusState.identity.slice(0, 120));
  }
  expect(new Set(focusTrail).size).toBeGreaterThan(8);
});

test("forced colors, increased contrast, reduced motion, and reduced transparency take effect", async ({ page }, testInfo) => {
  const session = await page.context().newCDPSession(page);
  await session.send("Emulation.setEmulatedMedia", {
    features: [
      { name: "forced-colors", value: "active" },
      { name: "prefers-contrast", value: "more" },
      { name: "prefers-reduced-motion", value: "reduce" },
      { name: "prefers-reduced-transparency", value: "reduce" },
    ],
  });
  await page.setViewportSize({ width: 540, height: 360 });
  await page.goto("/?platform=macos");

  const preferences = await page.evaluate(() => ({
    forcedColors: matchMedia("(forced-colors: active)").matches,
    increasedContrast: matchMedia("(prefers-contrast: more)").matches,
    reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
    reducedTransparency: matchMedia("(prefers-reduced-transparency: reduce)").matches,
    topBarBackdrop: getComputedStyle(document.querySelector(".top-bar")!).backdropFilter,
    panelBorderWidth: getComputedStyle(document.querySelector(".panel")!).borderTopWidth,
  }));
  expect(preferences).toMatchObject({
    forcedColors: true,
    increasedContrast: true,
    reducedMotion: true,
    reducedTransparency: true,
    topBarBackdrop: "none",
    panelBorderWidth: "2px",
  });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await retainScreenshot(page, testInfo, "forced-colors-reduced-effects-200-percent");
});

test("grouped Settings has no automated accessibility violations", async ({ page }) => {
  await page.setViewportSize({ width: 1080, height: 720 });
  await page.goto("/");
  await page.getByRole("button", { name: "Open settings" }).click();

  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Settings sections" })).toBeVisible();

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("Appearance preview persists and remains accessible at 200% zoom equivalent", async ({ page }) => {
  await page.setViewportSize({ width: 1080, height: 720 });
  await page.goto("/");
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: /Appearance Theme and visual behavior/ }).click();
  await page.setViewportSize({ width: 540, height: 360 });
  await page.getByRole("group", { name: "Theme mode" }).getByRole("button", { name: /Light/ }).click();
  await page.getByRole("group", { name: "Theme accent" }).getByRole("button", { name: "Violet" }).click();
  await page.getByRole("group", { name: "Theme preset" }).getByRole("button", { name: /Verdant/ }).click();

  await expect(page.getByRole("heading", { name: "Representative controls and states" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("html")).toHaveAttribute("data-accent", "violet");
  await expect(page.locator("html")).toHaveAttribute("data-theme-preset", "verdant");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  const dimensions = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);

  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme-preset", "verdant");
});

test("optional local companion is static, persistent, accessible, and immediately disableable", async ({ page }) => {
  await page.setViewportSize({ width: 1080, height: 720 });
  await page.goto("/");
  await expect(page.getByRole("complementary", { name: "Pandora companion" })).toHaveCount(0);
  await page.getByRole("button", { name: "Open settings" }).click();
  await page.getByRole("button", { name: /Appearance Theme and visual behavior/ }).click();
  await page.getByRole("group", { name: "Companion visibility" }).getByRole("button", { name: "On" }).click();
  await page.getByRole("group", { name: "Companion preview state" }).getByRole("button", { name: "working" }).click();
  await page.emulateMedia({ reducedMotion: "reduce" });
  expect(await page.locator(".companion-preview img").evaluate((image) => getComputedStyle(image).animationName)).toBe("none");
  await page.getByRole("group", { name: "Companion motion" }).getByRole("button", { name: "static" }).click();
  await page.getByRole("group", { name: "Companion preview state" }).getByRole("button", { name: "waiting" }).click();

  const companion = page.getByRole("complementary", { name: "Pandora companion" });
  await expect(companion).toBeVisible();
  await expect(companion.getByRole("status")).toContainText("Idle and ready");
  await expect(page.getByText("Waiting for an exact approval")).toBeVisible();
  expect(await companion.locator("img").evaluate((image: HTMLImageElement) => image.complete && image.naturalWidth > 0)).toBe(true);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  await page.reload();
  const restored = page.getByRole("complementary", { name: "Pandora companion" });
  await expect(restored).toHaveClass(/motion-static/);
  await restored.getByRole("button", { name: "Disable Pandora companion" }).click();
  await expect(restored).toHaveCount(0);
});

for (const viewport of viewports) {
  for (const layout of layouts) {
    test(`${layout.label} avoids horizontal clipping at ${viewport.label}`, async ({ page }, testInfo) => {
      await page.addInitScript(({ open, placement }) => {
        window.localStorage.setItem("pandora.desktop.dock.open", String(open));
        window.localStorage.setItem("pandora.desktop.dock.placement", placement);
        window.localStorage.setItem("pandora.desktop.dock.size", "expanded");
      }, layout);
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto("/");

      await expect(page.getByRole("main")).toBeVisible();
      await expect(page.getByRole("navigation", { name: "Pandora navigation" })).toBeVisible();
      await expect(page.locator(".command-layout")).toHaveAttribute("data-dock-placement", layout.open ? layout.placement : "closed");

      const dimensions = await page.evaluate(() => ({
        clientWidth: document.documentElement.clientWidth,
        scrollWidth: document.documentElement.scrollWidth,
      }));
      expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
      await retainScreenshot(page, testInfo, `${layout.label}-${viewport.label}`);
    });
  }
}
