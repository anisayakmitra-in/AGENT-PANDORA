import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const viewports = [
  { label: "minimum desktop window", width: 1080, height: 720 },
  { label: "200% zoom equivalent", width: 540, height: 360 },
];

test("Command Center has no automated accessibility violations", async ({ page }) => {
  await page.setViewportSize({ width: 1080, height: 720 });
  await page.goto("/");

  await expect(page.getByRole("main")).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Pandora navigation" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Search" })).toBeVisible();

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

for (const viewport of viewports) {
  test(`Command Center avoids horizontal clipping at ${viewport.label}`, async ({ page }) => {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await page.goto("/");

    await expect(page.getByRole("main")).toBeVisible();
    await expect(page.getByRole("navigation", { name: "Pandora navigation" })).toBeVisible();

    const dimensions = await page.evaluate(() => ({
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
    }));
    expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
  });
}
