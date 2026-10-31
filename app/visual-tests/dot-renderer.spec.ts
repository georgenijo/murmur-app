import { expect, test } from '@playwright/test';

test('GPU paints the real field at both backing scales and falls back on context loss', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=voice-island&placement=below');
  const canvas = page.locator('[data-variant="recording"] canvas');
  await expect(canvas).toHaveAttribute('data-renderer', 'gpu');
  const metrics = await page.evaluate(async () => {
    const source = '/src/components/overlay/';
    const { DotScope } = await import(source + 'dotScope.ts');
    const { prepareDots, paintDots } = await import(source + 'DotField.tsx');
    const { createGpuDotRenderer } = await import(source + 'gpuDotRenderer.ts');
    const results = [];
    for (const ratio of [1, 2]) for (const reduced of [false, true]) {
      for (const stage of ['recording', 'processing', 'done', 'idle']) {
        for (const style of ['sweep', 'flow', 'pulse']) {
          const gpuCanvas = document.createElement('canvas');
          const reference = document.createElement('canvas');
          gpuCanvas.width = reference.width = 112 * ratio;
          gpuCanvas.height = reference.height = 26 * ratio;
          const gpu = createGpuDotRenderer(gpuCanvas);
          if (!gpu) throw new Error('WebGL initialization failed');
          const a = new DotScope(), b = new DotScope();
          for (let f = 0; f < 80; f++) {
            for (const scope of [a, b]) {
              scope.advance(1 / 60, f < 70 ? 'recording' : stage, -34, reduced);
              scope.sample(1 / 60, style, reduced);
            }
          }
          prepareDots(a, 1 / 60, style, reduced); gpu.paint(a, style);
          const ctx = reference.getContext('2d')!;
          ctx.scale(ratio, ratio); paintDots(ctx, b, 1 / 60, style, reduced);
          // Read in the same task, before WebGL's default buffer is discarded.
          const gpuPixels = document.createElement('canvas');
          gpuPixels.width = reference.width; gpuPixels.height = reference.height;
          const read = gpuPixels.getContext('2d')!;
          read.drawImage(gpuCanvas, 0, 0);
          const actual = read.getImageData(0, 0, reference.width, reference.height).data;
          const expected = ctx.getImageData(0, 0, reference.width, reference.height).data;
          let alphaError = 0, alphaSum = 0, painted = 0;
          for (let n = 3; n < actual.length; n += 4) {
            alphaError += Math.abs(actual[n] - expected[n]); alphaSum += expected[n];
            if (actual[n]) painted++;
          }
          results.push({ ratio, stage, style, reduced, painted,
            meanAlphaError: alphaError / (actual.length / 4),
            relativeCoverageError: alphaSum ? alphaError / alphaSum : 0,
            simulationEqual: a.dots.every((v: number, n: number) => v === b.dots[n]),
          });
          gpu.dispose();
          gpuCanvas.getContext('webgl')!.getExtension('WEBGL_lose_context')?.loseContext();
        }
      }
    }
    return results;
  });
  for (const result of metrics) {
    expect(result.simulationEqual).toBe(true);
    expect(result.painted).toBeGreaterThan(0);
    // Analytic circle antialiasing differs from Canvas path coverage at edges.
    // Keep total plate error bounded while the simulation remains identical.
    expect(result.meanAlphaError, JSON.stringify(result)).toBeLessThan(5);
  }
  await canvas.evaluate((element: HTMLCanvasElement) => {
    const extension = element.getContext('webgl')!.getExtension('WEBGL_lose_context');
    if (!extension) throw new Error('Context loss extension unavailable');
    extension.loseContext();
  });
  await expect(page.locator('[data-variant="recording"] canvas')).toHaveAttribute('data-renderer', 'canvas');
  await expect(page.locator('[data-variant="recording"] canvas')).toHaveAttribute('width', '112');
});

test('blocked WebGL still renders with the existing Canvas backend', async ({ page }) => {
  await page.addInitScript(() => {
    const original = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (kind: string, ...args: unknown[]) {
      if (kind === 'webgl') return null;
      return Reflect.apply(original, this, [kind, ...args]);
    } as typeof original;
  });
  await page.goto('/visual-fixtures.html?state=voice-island&placement=below');
  const canvas = page.locator('[data-variant="recording"] canvas');
  await expect(canvas).toHaveAttribute('data-renderer', 'canvas');
  expect(await canvas.evaluate((element: HTMLCanvasElement) => {
    const pixels = element.getContext('2d')!.getImageData(0, 0, element.width, element.height).data;
    return pixels.some((v, n) => n % 4 === 3 && v > 0);
  })).toBe(true);
});
