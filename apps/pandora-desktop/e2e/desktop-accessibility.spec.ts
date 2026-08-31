import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const viewports = [
  { label: "minimum desktop window", width: 1080, height: 720 },
  { label: "200% zoom equivalent", width: 540, height: 360 },
];

const layouts = [
  { label: "right dock", open: true, placement: "right" },
  { label: "bottom dock", open: true, placement: "bottom" },
  { label: "hidden dock", open: false, placement: "right" },
] as const;

test("Command Center has no automated accessibility violations", async ({ page }) => {
  await page.setViewportSize({ width: 1080, height: 720 });
  await page.goto("/");

  await expect(page.getByRole("main")).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Pandora navigation" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Search", exact: true })).toBeVisible();

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
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

for (const viewport of viewports) {
  for (const layout of layouts) {
    test(`${layout.label} avoids horizontal clipping at ${viewport.label}`, async ({ page }) => {
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
    });
  }
}
