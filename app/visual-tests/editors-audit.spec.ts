import { expect, test, type Locator } from '@playwright/test';

async function readableText(locator: Locator) {
  return locator.evaluate(element => {
    // Canvas resolves the browser's actual color syntax, including Tailwind's
    // oklch/color-mix values. Composite every ancestor rather than assuming the
    // fixture's desktop color is the review surface.
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 1;
    const context = canvas.getContext('2d', { willReadFrequently: true })!;
    const rgba = (color: string) => {
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = color;
      context.fillRect(0, 0, 1, 1);
      const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
      return [r, g, b, a / 255];
    };
    const over = (front: number[], back: number[]) => front.slice(0, 3).map((value, i) => value * front[3] + back[i] * (1 - front[3]));
    const ancestors: Element[] = [];
    for (let node: Element | null = element; node; node = node.parentElement) ancestors.unshift(node);
    let background = [255, 255, 255];
    for (const node of ancestors) background = over(rgba(getComputedStyle(node).backgroundColor), background);
    const style = getComputedStyle(element);
    const foreground = over(rgba(style.color), background);
    const luminance = (color: number[]) => color.map(value => {
      const channel = value / 255;
      return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
    }).reduce((sum, value, i) => sum + value * [0.2126, 0.7152, 0.0722][i], 0);
    const light = luminance(foreground);
    const dark = luminance(background);
    return {
      text: element.textContent, fontSize: style.fontSize, fontWeight: style.fontWeight,
      declaredColor: style.color, foreground, background,
      contrast: (Math.max(light, dark) + 0.05) / (Math.min(light, dark) + 0.05),
    };
  });
}

for (const appearance of ['light', 'dark'] as const) {
  test(`command editor keeps actions visible at 720×560 in ${appearance}`, async ({ page }) => {
    await page.setViewportSize({ width: 720, height: 560 });
    await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=commands&appearance=${appearance}`);
    await page.getByRole('button', { name: 'New command…', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Create voice command', exact: true });
    const save = dialog.getByRole('button', { name: 'Save command', exact: true });
    const cancel = dialog.getByRole('button', { name: 'Cancel', exact: true });
    for (const action of [save, cancel]) {
      await expect(action).toBeInViewport({ ratio: 1 });
      const box = (await action.boundingBox())!;
      expect(box.y + box.height).toBeLessThanOrEqual(560);
    }
    // The last editable field can scroll into view without covering the pinned footer.
    const field = dialog.getByRole('textbox', { name: 'Voice command test phrase', exact: true });
    await field.focus();
    await expect(field).toBeInViewport({ ratio: 1 });
    await expect(save).toBeInViewport({ ratio: 1 });
    await dialog.getByRole('textbox', { name: 'Voice command phrase', exact: true }).fill('Disposable draft');
    await page.keyboard.press('Escape');
    await dialog.getByRole('button', { name: 'Keep editing', exact: true }).click();
    await expect(dialog.getByRole('textbox', { name: 'Voice command phrase', exact: true })).toHaveValue('Disposable draft');
    await cancel.click();
    await dialog.getByRole('button', { name: 'Discard changes', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'New command…', exact: true })).toBeFocused();
    await page.getByRole('button', { name: 'Open Voice commands & spoken formatting', exact: true }).click();
    const globalSwitch = page.getByRole('switch', { name: 'Voice commands & spoken formatting', exact: true });
    await expect(globalSwitch).toBeVisible();
    await expect(globalSwitch).toBeFocused();
    await expect(globalSwitch).toHaveAttribute('aria-checked', 'false');
    await expect(page.getByRole('button', { name: 'New command…', exact: true })).toHaveCount(0);
  });

  test(`live theme editor keeps its real actions visible at 720×560 in ${appearance}`, async ({ page }) => {
    await page.setViewportSize({ width: 720, height: 560 });
    await page.goto(`/visual-fixtures.html?state=settings-appearance&appearance=${appearance}`);
    await page.getByRole('button', { name: 'Create theme', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Create theme', exact: true });
    for (const name of ['Reset to Sonic', 'Export', 'Done']) {
      const action = dialog.getByRole('button', { name, exact: true });
      await expect(action).toBeInViewport({ ratio: 1 });
      const box = (await action.boundingBox())!;
      expect(box.y + box.height).toBeLessThanOrEqual(560);
    }
    await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toHaveCount(0);
    const contrast = dialog.getByRole('slider', { name: 'Contrast', exact: true });
    await contrast.focus();
    await expect(contrast).toBeInViewport({ ratio: 1 });
    await expect(dialog.getByRole('button', { name: 'Done', exact: true })).toBeInViewport({ ratio: 1 });
    await dialog.getByRole('button', { name: 'Done', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Create theme', exact: true })).toBeFocused();
  });

  test(`query secondary text has measured contrast on its opaque ${appearance} surface`, async ({ page }, testInfo) => {
    await page.goto(`/visual-fixtures.html?state=query-review&appearance=${appearance}`);
    const surface = page.locator('.query-review-surface');
    await expect(surface).toBeVisible();
    const targets = [
      surface.locator('.query-review-status'),
      surface.locator('.query-review-details summary'),
      surface.getByRole('button', { name: 'Close', exact: true }),
      surface.getByRole('button', { name: 'Ask follow-up', exact: true }),
    ];
    const evidence = [];
    for (const target of targets) {
      const measured = await readableText(target);
      evidence.push(measured);
      expect(measured.contrast, measured.text ?? '').toBeGreaterThanOrEqual(4.5);
    }
    await testInfo.attach('computed-query-text.json', { body: JSON.stringify(evidence, null, 2), contentType: 'application/json' });
  });
}

for (const backdrop of ['black', 'white']) {
  test(`rewrite labels shortcuts and diff have measured contrast over ${backdrop}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 420, height: 360 });
    await page.goto('/transform-review.html?mock=ready');
    await page.evaluate(color => { document.documentElement.style.background = color; document.body.style.background = color; }, backdrop);
    const surface = page.locator('.transform-review-surface');
    await expect(surface).toHaveCSS('opacity', '1');
    await expect(surface).toHaveCSS('background-color', 'rgb(20, 20, 20)');
    expect(await surface.evaluate(element => {
      const style = getComputedStyle(element);
      return style.backdropFilter === 'none' && !/blur/.test(style.getPropertyValue('-webkit-backdrop-filter'));
    })).toBe(true);
    await testInfo.attach(`opaque-rewrite-${backdrop}.png`, { body: await page.screenshot(), contentType: 'image/png' });
    const targets = [
      surface.getByText('On-device', { exact: true }),
      surface.getByText('⌘R', { exact: true }),
      surface.getByText('⏎', { exact: true }),
      surface.getByText('esc', { exact: true }),
      surface.getByRole('button', { name: /Retry/ }),
      surface.getByRole('button', { name: /Approve/ }),
      surface.getByRole('button', { name: /Cancel/ }),
      surface.locator('.line-through').first(),
      surface.locator('[class*="text-emerald-400"]').first(),
    ];
    const evidence = [];
    for (const target of targets) {
      const measured = await readableText(target);
      evidence.push(measured);
      expect(measured.contrast, measured.text ?? '').toBeGreaterThanOrEqual(4.5);
    }
    // Shortcut backgrounds change on hover; the same contract still applies.
    for (const [button, shortcut] of [['Retry', '⌘R'], ['Approve', '⏎'], ['Cancel', 'esc']]) {
      await surface.getByRole('button', { name: new RegExp(button) }).hover();
      // Wait for the actual hover transition, not a pre-transition sample.
      await page.waitForTimeout(200);
      const hovered = await readableText(surface.getByText(shortcut, { exact: true }));
      evidence.push(hovered);
      expect(hovered.contrast).toBeGreaterThanOrEqual(4.5);
    }
    await testInfo.attach('computed-rewrite-text.json', { body: JSON.stringify(evidence, null, 2), contentType: 'application/json' });
  });
}
