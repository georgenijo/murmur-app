import { expect, test } from '@playwright/test';

const states = ['idle', 'recording', 'processing', 'update-recovering', 'settings'] as const;
const appearances = ['light', 'dark'] as const;
const dashboardThemeMatrix = [
  { id: 'sonic-light', appearance: 'light', theme: null },
  { id: 'sonic-dark', appearance: 'dark', theme: null },
  { id: 'open-vsx-low-contrast', appearance: 'light', theme: 'open-vsx-low-contrast' },
  { id: 'open-vsx-high-saturation', appearance: 'dark', theme: 'open-vsx-high-saturation' },
] as const;

for (const appearance of appearances) {
  for (const state of states) {
    test(`${appearance} ${state} matches the dashboard at native 880x720`, async ({ page }) => {
      const pageErrors: string[] = [];
      page.on('pageerror', (error) => pageErrors.push(error.message));
      await page.emulateMedia({ colorScheme: appearance });
      await page.goto(`/visual-fixtures.html?state=${state}&appearance=${appearance}`);
      expect(pageErrors).toEqual([]);
      const fixture = page.locator('[data-visual-ready="true"]');
      await expect(fixture).toBeVisible();
      await expect(fixture).toHaveScreenshot(`${appearance}-${state}.png`);
    });
  }
}

for (const themeCase of dashboardThemeMatrix) {
  for (const destination of ['home', 'insights'] as const) {
    test(`${themeCase.id} keeps ${destination} hierarchy discernible`, async ({ page }) => {
      const state = destination === 'home' ? 'idle' : 'insights';
      const theme = themeCase.theme ? `&theme=${themeCase.theme}` : '';
      await page.emulateMedia({ colorScheme: themeCase.appearance });
      await page.goto(`/visual-fixtures.html?state=${state}&appearance=${themeCase.appearance}${theme}`);

      const fixture = page.locator('[data-visual-ready="true"]');
      await expect(fixture).toHaveAttribute('data-theme-fixture', themeCase.theme ?? 'sonic');
      await expect(fixture).toHaveScreenshot(`theme-${themeCase.id}-${destination}.png`);
    });
  }
}

test('selected history filters remain selected while hovered', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/visual-fixtures.html?state=idle&appearance=dark');
  const mic = page.getByRole('button', { name: 'Mic' });
  await mic.hover();
  await mic.click();
  await expect(mic).toHaveAttribute('aria-pressed', 'true');

  await expect.poll(() => mic.evaluate((element) => {
    const selected = getComputedStyle(element);
    const probe = document.createElement('span');
    probe.style.color = 'var(--murmur-on-surface)';
    document.body.appendChild(probe);
    const expectedBackground = getComputedStyle(probe).color;
    probe.style.color = 'var(--murmur-background)';
    const expectedForeground = getComputedStyle(probe).color;
    probe.remove();
    return {
      backgroundMatches: selected.backgroundColor === expectedBackground,
      foregroundMatches: selected.color === expectedForeground,
    };
  })).toEqual({ backgroundMatches: true, foregroundMatches: true });
});

test('copied middle transcript keeps its geometry and reserves feedback space', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-write'], { origin: 'http://127.0.0.1:1420' });
  await page.setViewportSize({ width: 880, height: 720 });
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });
  });
  const middleCard = page.locator('.home-history .transcript-card').nth(1);
  const transcript = middleCard.locator('.transcript-text');
  const feedback = middleCard.locator('.transcript-copy-feedback');
  const before = await middleCard.boundingBox();

  await expect(middleCard).toHaveAttribute('data-day-end', 'false');
  await middleCard.click();
  await expect(middleCard).toHaveAttribute('data-copied', 'true');
  await expect(feedback).toHaveText('Copied');

  const [after, transcriptBox, feedbackBox, transcriptPaddingRight, successColor] = await Promise.all([
    middleCard.boundingBox(),
    transcript.boundingBox(),
    feedback.boundingBox(),
    transcript.evaluate((element) => parseFloat(getComputedStyle(element).paddingRight)),
    middleCard.evaluate(() => {
      const probe = document.createElement('span');
      probe.style.color = 'var(--murmur-success)';
      document.body.appendChild(probe);
      const color = getComputedStyle(probe).color;
      probe.remove();
      return color;
    }),
  ]);
  expect(after).toEqual(before);
  expect(feedbackBox!.x).toBeGreaterThanOrEqual(
    transcriptBox!.x + transcriptBox!.width - transcriptPaddingRight,
  );
  await expect.poll(() => middleCard.evaluate((element) => getComputedStyle(element).borderColor))
    .toBe(successColor);
  expect(await middleCard.evaluate((element) => getComputedStyle(element).boxShadow))
    .toContain(successColor);
  await expect(middleCard).toHaveScreenshot('light-history-copy-middle.png');

  const newestCard = page.locator('.home-history .transcript-card').first();
  const teach = newestCard.getByRole('button', { name: 'Correct & Teach' });
  await newestCard.click();
  const [newestFeedbackBox, teachBox] = await Promise.all([
    newestCard.locator('.transcript-copy-feedback').boundingBox(),
    teach.boundingBox(),
  ]);
  expect(newestFeedbackBox!.y + newestFeedbackBox!.height).toBeLessThanOrEqual(teachBox!.y);

  await page.setViewportSize({ width: 680, height: 720 });
  await page.reload();
  const narrowCard = page.locator('.home-history .transcript-card').nth(1);
  const narrowTranscript = narrowCard.locator('.transcript-text');
  const narrowFeedback = narrowCard.locator('.transcript-copy-feedback');
  await narrowCard.click();
  await expect(narrowFeedback).toHaveText('Copied');
  const [narrowTextBox, narrowFeedbackBox, narrowPaddingRight] = await Promise.all([
    narrowTranscript.boundingBox(),
    narrowFeedback.boundingBox(),
    narrowTranscript.evaluate((element) => parseFloat(getComputedStyle(element).paddingRight)),
  ]);
  expect(narrowFeedbackBox!.x).toBeGreaterThanOrEqual(
    narrowTextBox!.x + narrowTextBox!.width - narrowPaddingRight,
  );
  await expect(narrowCard).toHaveScreenshot('light-history-copy-middle-narrow.png');

  await page.setViewportSize({ width: 520, height: 720 });
  await page.reload();
  const overflowCard = page.locator('.home-history .transcript-card').nth(1);
  await overflowCard.locator('.transcript-text').click({ position: { x: 4, y: 4 } });
  await overflowCard.locator('.transcript-text').evaluate((element) => {
    element.textContent = 'A long copied transcript keeps expanding across the row without hiding its controls. '.repeat(8);
  });
  await page.setViewportSize({ width: 519, height: 720 });
  const expand = overflowCard.getByRole('button', { name: 'Show more' });
  await expect(expand).toBeVisible();
  const [overflowFeedbackBox, expandBox] = await Promise.all([
    overflowCard.locator('.transcript-copy-feedback').boundingBox(),
    expand.boundingBox(),
  ]);
  expect(overflowFeedbackBox!.y + overflowFeedbackBox!.height).toBeLessThanOrEqual(expandBox!.y);
});

test('the transcript search placeholder fits beside its shortcut badge', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=idle&appearance=dark');
  const search = page.getByRole('searchbox', { name: 'Search transcripts' });
  const fit = await search.evaluate((input: HTMLInputElement) => {
    const style = getComputedStyle(input);
    const canvas = document.createElement('canvas');
    const context = canvas.getContext('2d')!;
    context.font = style.font;
    const textWidth = context.measureText(input.placeholder).width;
    const availableWidth = input.clientWidth
      - parseFloat(style.paddingLeft)
      - parseFloat(style.paddingRight);
    return { textWidth, availableWidth };
  });

  expect(fit.textWidth).toBeLessThanOrEqual(fit.availableWidth);
});

test('the window header flows into the history toolbar without a divider', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  const header = page.locator('.ui-window-header');
  await expect(header).toBeVisible();
  await expect.poll(() => header.evaluate((element) => (
    getComputedStyle(element).borderBottomWidth
  ))).toBe('0px');
  await expect(page.locator('.ui-window-wordmark')).toHaveCount(0);
  await expect.poll(() => page.locator('.home-brand').evaluate((element) => (
    element.getBoundingClientRect().left
  ))).toBe(8);
});

test('native default keeps the approved three-column dashboard geometry', async ({ page }) => {
  await page.setViewportSize({ width: 880, height: 720 });
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');

  const sidebar = page.locator('.home-sidebar');
  const main = page.locator('.home-dashboard-main');
  const rail = page.locator('.home-insights-rail');
  const [sidebarBox, mainBox, railBox] = await Promise.all([
    sidebar.boundingBox(),
    main.boundingBox(),
    rail.boundingBox(),
  ]);

  expect(sidebarBox?.width).toBe(160);
  expect(mainBox?.width).toBeGreaterThan(450);
  expect(railBox?.width).toBe(200);
  expect(railBox?.y).toBe(mainBox?.y);
  await expect(page.getByText('This month', { exact: true })).toBeVisible();
  await expect(page.getByText('Voice profile', { exact: true })).toBeVisible();
  await expect(page.locator('.home-history .history-date-label')).toHaveCount(1);
});

for (const state of ['recording', 'update-recovering', 'settings'] as const) {
  test(`${state} title-bar controls share the native traffic-light centerline`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=${state}&appearance=light`);

    const header = page.locator('.ui-window-header');
    const items = state === 'settings'
      ? [
          header.locator('.ui-window-wordmark'),
          header.getByText('Settings', { exact: true }),
          header.getByRole('button', { name: 'Done' }),
        ]
      : [
          header.getByTestId('main-status-chip'),
          header.getByRole('button', { name: 'Open settings' }),
        ];
    const [headerBox, centers] = await Promise.all([
      header.boundingBox(),
      Promise.all(items.map((item) => item.evaluate((element) => {
        const box = element.getBoundingClientRect();
        return box.top + box.height / 2;
      }))),
    ]);

    expect(Math.max(...centers) - Math.min(...centers)).toBeLessThanOrEqual(0.5);
    expect(headerBox).not.toBeNull();
    expect((headerBox!.y + headerBox!.height / 2) - centers[0]).toBeCloseTo(2, 1);
  });
}

test('update discovery cannot expand or wrap the recovering header', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=update-recovering&appearance=light');

  const header = page.locator('.ui-window-header');
  const update = page.getByTestId('update-indicator');
  const record = page.getByTestId('home-record-button');

  await expect(header).toBeVisible();
  await expect(update).toHaveAccessibleName('Murmur v0.27.1 is available. View update');
  await expect(record).toHaveAccessibleName('Recovering microphone');
  await expect(record).toBeDisabled();

  const geometry = await Promise.all([
    header.boundingBox(),
    update.boundingBox(),
    record.boundingBox(),
  ]);
  const [headerBox, updateBox, recordBox] = geometry;

  expect(updateBox?.width).toBeLessThanOrEqual(26);
  expect(updateBox?.height).toBeLessThanOrEqual(26);
  expect(recordBox?.height).toBe(36);
  expect(headerBox?.height).toBe(42);
});

test('settings editors preserve the primary hierarchy and provide a real back action', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings&appearance=light');
  await page.getByRole('button', { name: 'Text' }).click();
  await page.getByRole('button', { name: /^Aliases\b/ }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  await expect(page.getByRole('navigation', { name: 'Settings pages' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Text & Vocabulary', exact: true })).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('navigation', { name: 'Settings editors' })).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Aliases', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back to Text settings' })).toBeVisible();
  await expect(fixture).toHaveScreenshot('light-settings-aliases.png');

  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: 'Text & Vocabulary' })).toBeVisible();
});

test('recording settings make the live automatic microphone choice explicit', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings&appearance=light');
  await page.getByRole('button', { name: 'Recording', exact: true }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  await expect(page.getByRole('combobox', { name: 'Microphone input' })).toContainText(
    'Follow macOS Default — MacBook Pro Microphone',
  );
  await expect(page.getByText(/Docking, undocking, or changing the system input/)).toBeVisible();
  await expect(fixture).toHaveScreenshot('light-settings-recording-auto-microphone.png');
});

test('appearance matches the compact collection-card layout', async ({ page }) => {
  const source = {
    kind: 'open-vsx',
    extensionId: 'h1dr0n.claude-theme',
    version: '1.0.0',
    license: 'MIT',
  } as const;
  const collection = { id: 'open-vsx:h1dr0n.claude-theme', label: 'Claude Theme' };
  const entry = (
    id: string,
    label: string,
    modes: Array<'light' | 'dark'>,
    theme: Record<string, unknown>,
  ) => ({
    version: 1,
    id,
    label,
    modes,
    theme: { version: 1, presetId: 'custom', ...theme },
    source,
    collection,
  });
  const library = {
    version: 1,
    revision: 1,
    themes: [
      entry('claude-classic', 'Claude Classic', ['light', 'dark'], {
        light: { background: '#f1efe7', foreground: '#141413', accent: '#0060a4' },
        dark: { background: '#1a1d23', foreground: '#d6d6d6', accent: '#74a9d8' },
      }),
      entry('claude-dusk', 'Claude Dusk (Deep Slate)', ['dark'], {
        dark: { background: '#1a1d23', foreground: '#d6d6d6', accent: '#74a9d8' },
      }),
      entry('claude-midnight', 'Claude Midnight (OLED Black)', ['dark'], {
        dark: { background: '#000000', foreground: '#f5f5f5', accent: '#74a9d8' },
      }),
      entry('claude-midnight-light', 'Claude Midnight Light (Pure High-Contrast)', ['light'], {
        light: { background: '#ffffff', foreground: '#000000', accent: '#000000' },
      }),
    ],
  };
  await page.addInitScript((value) => {
    localStorage.setItem('murmur-theme-library', JSON.stringify(value));
  }, library);
  await page.goto('/visual-fixtures.html?state=settings-appearance&appearance=light');
  await page.getByRole('button', { name: 'Appearance', exact: true }).click();
  await page.getByRole('button', { name: 'Use Claude Theme theme' }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  const sonicCard = page.locator('[data-theme-collection="Sonic"]');
  const claudeCard = page.locator('[data-theme-collection="Claude Theme"]');
  await expect(claudeCard).toHaveCount(1);
  await expect(sonicCard).toHaveCSS('width', '208px');
  await expect(sonicCard).toHaveCSS('height', '94px');
  await expect(claudeCard).toHaveCSS('width', '208px');
  await expect(claudeCard).toHaveCSS('height', '94px');
  await expect(claudeCard.locator('button[aria-label*="light variant"][aria-pressed="true"]')).toBeVisible();
  await expect(claudeCard.locator('button[aria-label*="dark variant"][aria-pressed="true"]')).toBeVisible();
  await expect(page.locator('section[aria-labelledby="active-theme-heading"]')).toHaveCount(0);
  await expect(claudeCard.getByText('Active', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('radio', { name: /system/i })).not.toContainText('Active');
  await expect(page.getByText('Partly active')).toHaveCount(0);
  await expect(page.getByText('Choose dark style')).toHaveCount(0);
  await expect(page.getByText('Import a file or browse Open VSX')).toHaveCount(0);
  await expect(fixture).toHaveScreenshot('light-settings-appearance.png');

  await page.getByRole('radio', { name: /dark/i }).click();
  await expect(claudeCard.locator('button[aria-label*="dark variant"][aria-pressed="true"]')).toBeVisible();
  await expect(fixture).toHaveScreenshot('dark-settings-appearance.png');

  const cardHeight = await claudeCard.evaluate((element) => element.getBoundingClientRect().height);
  await claudeCard.locator('button[aria-label^="Choose dark variant"]').hover();
  const midnight = page.getByRole('button', { name: /Use .*Midnight.* for dark mode/ });
  await expect(midnight).toBeVisible();
  await expect.poll(() => claudeCard.evaluate((element) => element.getBoundingClientRect().height)).toBe(cardHeight);
  await midnight.click();
  await expect(claudeCard.locator('button[aria-label*="dark variant"][aria-label*="Midnight"]')).toBeVisible();
  await expect.poll(() => claudeCard.evaluate((element) => element.getBoundingClientRect().height)).toBe(cardHeight);
});

test('the main recording waveform reacts to audio without pulse animation', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=recording&appearance=dark');
  const waveform = page.locator('.home-record-waveform');

  await expect(waveform.locator('span')).toHaveCount(5);
  await expect(waveform.locator('.animate-pulse')).toHaveCount(0);
  const heights = await waveform.locator('span').evaluateAll((bars) => (
    bars.map((bar) => getComputedStyle(bar).height)
  ));
  expect(new Set(heights).size).toBeGreaterThan(1);
});

test('the sidebar opens a real expanded Insights view', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  await page.getByRole('button', { name: 'Insights', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Insights' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Developing' })).toBeVisible();
  await expect(page.getByText(/voice-training or confidence score/i)).toBeVisible();
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-insights.png');
});

test('secondary destinations share Back behavior and restore focus to Home navigation', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  const home = page.getByRole('button', { name: 'Home', exact: true });

  for (const destination of ['Notetaker', 'Queries', 'Insights'] as const) {
    await page.getByRole('button', { name: destination, exact: true }).click();
    await expect(page.getByRole('heading', { name: destination, exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Back to Home', exact: true }).click();
    await expect(home).toHaveAttribute('aria-current', 'page');
    await expect(home).toBeFocused();
  }
});

test('dashboard charts keep tooltip, plot, and seven weekday labels in stable regions', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=insights&appearance=light');
  const chart = page.locator('figure[aria-label="Words per day bar chart"]');
  const tooltip = chart.locator('.ui-day-chart-tooltip');
  const plot = chart.locator('.ui-day-chart-plot');
  const labels = chart.locator('.ui-day-chart-axis span');
  const marks = chart.locator('.ui-day-chart-bar');
  await expect(labels).toHaveCount(7);
  await expect(marks).toHaveCount(7);

  const before = await Promise.all([tooltip.boundingBox(), plot.boundingBox(), chart.boundingBox()]);
  await marks.nth(2).focus();
  await expect(tooltip).toContainText('words');
  const after = await Promise.all([tooltip.boundingBox(), plot.boundingBox(), chart.boundingBox()]);
  expect(after).toEqual(before);

  const [plotBox, labelBoxes] = await Promise.all([
    plot.boundingBox(),
    labels.evaluateAll((elements) => elements.map((element) => {
      const box = element.getBoundingClientRect();
      return { left: box.left, right: box.right, top: box.top };
    })),
  ]);
  expect(labelBoxes.every((box) => box.top >= plotBox!.y + plotBox!.height)).toBe(true);
  expect(labelBoxes.every((box, index) => index === 0 || box.left >= labelBoxes[index - 1].right)).toBe(true);

  const lineChart = page.locator('figure[aria-label="Words-per-minute trend line"]');
  const [svgBox, pointCoordinates, targetBoxes] = await Promise.all([
    lineChart.locator('svg').boundingBox(),
    lineChart.locator('polyline').getAttribute('points'),
    lineChart.locator('.ui-day-chart-line-targets button').evaluateAll((elements) => (
      elements.map((element) => {
        const box = element.getBoundingClientRect();
        return { centerX: box.left + box.width / 2 };
      })
    )),
  ]);
  const lineXs = pointCoordinates!.split(' ').map((point) => Number(point.split(',')[0]));
  expect(lineXs).toHaveLength(7);
  lineXs.forEach((x, index) => {
    expect(svgBox!.x + (x / 100) * svgBox!.width).toBeCloseTo(targetBoxes[index].centerX, 1);
  });
});

test('dashboard actions keep hover, focus, active, and disabled states in an imported theme', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=idle&appearance=dark&theme=open-vsx-high-saturation');
  const action = page.getByRole('button', { name: 'View insights', exact: true });
  const initialBackground = await action.evaluate((element) => getComputedStyle(element).backgroundColor);
  await action.hover();
  await expect.poll(() => action.evaluate((element) => getComputedStyle(element).backgroundColor))
    .not.toBe(initialBackground);
  await action.focus();
  expect(await action.evaluate((element) => getComputedStyle(element).outlineStyle)).toBe('solid');
  const box = await action.boundingBox();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await page.mouse.down();
  expect(await action.evaluate((element) => getComputedStyle(element).transform)).not.toBe('none');
  await page.mouse.up();

  await page.goto('/visual-fixtures.html?state=processing&appearance=dark&theme=open-vsx-high-saturation');
  for (const disabled of [
    page.getByTestId('home-record-button'),
    page.getByRole('button', { name: 'Transcribe File', exact: true }),
  ]) {
    await expect(disabled).toBeDisabled();
    await expect(disabled).toHaveCSS('cursor', 'not-allowed');
    await expect(disabled).toHaveCSS('opacity', '1');
  }
});

test('the compact 720x560 home keeps actions and history reachable', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 560 });
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  await expect(page.getByRole('button', { name: 'Start recording' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Transcribe File' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Recent dictations' })).toBeVisible();
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-home-compact-720x560.png');
});
