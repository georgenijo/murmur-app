import { expect, test } from '@playwright/test';

test.use({ timezoneId: 'America/New_York' });

test.beforeEach(async ({ page }) => {
  // Keep calendar-based fixture data aligned with the checked-in screenshots.
  await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
});

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
  const mic = page.getByRole('tab', { name: 'Mic' });
  await mic.click();
  await mic.hover();
  await expect(mic).toHaveAttribute('aria-selected', 'true');

  const indicator = mic.locator('.history-filter-tab-indicator');
  await expect(indicator).toHaveCount(1);
  await expect.poll(() => indicator.evaluate((element) => {
    const probe = document.createElement('span');
    probe.style.color = 'var(--murmur-surface-container-lowest)';
    document.body.appendChild(probe);
    const expectedBackground = getComputedStyle(probe).color;
    probe.remove();
    return getComputedStyle(element).backgroundColor === expectedBackground;
  })).toBe(true);
});

test('copied transcripts keep their geometry and actions reachable', async ({ page, context }) => {
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
  const feedback = middleCard.locator('.transcript-copy-feedback');
  const copyAction = middleCard.locator('[data-action-id="copy"]');
  const before = await middleCard.boundingBox();

  await expect(middleCard).toHaveAttribute('data-day-end', 'false');
  await middleCard.click();
  await expect(middleCard).toHaveAttribute('data-copied', 'true');
  await expect(feedback).toHaveText('Copied');
  await expect(copyAction).toHaveText('Copied');
  expect(await middleCard.boundingBox()).toEqual(before);
  expect(await middleCard.evaluate((element) => getComputedStyle(element).boxShadow)).toBe('none');
  await expect(middleCard).toHaveScreenshot('light-history-copy-middle.png');

  const newestCard = page.locator('.home-history .transcript-card').first();
  await newestCard.getByRole('button', { name: 'More transcript actions' }).click();
  await expect(page.getByText('Correct & Teach', { exact: true }).last()).toBeVisible();
  await page.keyboard.press('Escape');

  await page.setViewportSize({ width: 680, height: 720 });
  await page.reload();
  const narrowCard = page.locator('.home-history .transcript-card').nth(1);
  await narrowCard.click();
  await expect(narrowCard.locator('[data-action-id="copy"]')).toHaveText('Copied');
  await expect(narrowCard).toHaveScreenshot('light-history-copy-middle-narrow.png');

  await page.setViewportSize({ width: 520, height: 720 });
  await page.reload();
  const overflowCard = page.locator('.home-history .transcript-card').nth(1);
  await overflowCard.locator('.transcript-text').click({ position: { x: 4, y: 4 } });
  await overflowCard.locator('.transcript-text').evaluate((element) => {
    element.textContent = 'A long copied transcript keeps expanding across the row without hiding its controls. '.repeat(8);
  });
  await page.setViewportSize({ width: 519, height: 720 });
  await expect(overflowCard.getByRole('button', { name: 'Show more' })).toBeVisible();
  await expect(overflowCard.locator('[data-action-id="copy"]')).toHaveText('Copied');
  const overflow = await overflowCard.evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(overflow.scrollWidth).toBe(overflow.clientWidth);
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
  expect(railBox?.width).toBe(212);
  expect(railBox?.y).toBe(mainBox?.y);
  await expect(page.getByText('This month', { exact: true })).toBeVisible();
  await expect(page.getByText('Voice profile', { exact: true })).toBeVisible();
  await expect(page.locator('.home-history .history-date-label')).toHaveCount(14);
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
          header.getByRole('button', { name: 'Open customization and settings' }),
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

  expect(updateBox?.width).toBeLessThanOrEqual(28);
  expect(updateBox?.height).toBeLessThanOrEqual(28);
  expect(recordBox?.height).toBe(30);
  expect(headerBox?.height).toBe(42);
});

test('customization hub stays legible and restores focus at native and narrow widths', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings&appearance=light');

  const fixture = page.locator('[data-visual-ready="true"]');
  const customizationList = page.getByRole('list', { name: 'Customization destinations' });
  const voiceCommands = customizationList.getByRole('button').filter({ hasText: 'Voice Commands' });
  await expect(page.getByRole('heading', { name: 'Customize Murmur' })).toBeVisible();
  await expect(customizationList.getByRole('button')).toHaveCount(4);
  await expect(fixture).toHaveScreenshot('light-settings-customization-hub.png');

  await voiceCommands.click();
  await expect(page.getByRole('heading', { name: 'Voice Commands', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Back to Customize' }).click();
  await expect(voiceCommands).toBeFocused();

  await page.setViewportSize({ width: 720, height: 560 });
  await expect(page.getByRole('heading', { name: 'Customize Murmur' })).toBeVisible();
  await expect(fixture).toHaveScreenshot('light-settings-customization-hub-narrow.png');
});

test('settings editors preserve the primary hierarchy and provide a real back action', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings&appearance=light');
  await page.getByRole('button', { name: 'Text & Vocabulary', exact: true }).click();
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

test('recording settings disclose approved Smart Auto microphones beneath their switch', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-smart-auto&appearance=light');
  await page.getByRole('button', { name: 'Recording', exact: true }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  const smartAuto = page.getByRole('switch', { name: 'Enable Smart Auto microphone selection' });
  await expect(smartAuto).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByText('Smart Auto will use MacBook Pro Microphone (preferred approved).')).toBeVisible();
  await expect(page.getByText('Approved: MacBook Pro Microphone, Anker USB Microphone.')).toBeVisible();
  const branch = smartAuto.locator('xpath=../following-sibling::*[1]');
  await expect(branch).toHaveAttribute('data-expanded', 'true');
  await expect(branch).toHaveAttribute('aria-hidden', 'false');
  await expect(fixture).toHaveScreenshot('light-settings-recording-smart-auto.png');
});

test('browser-site Mode rules disclose their exact privacy boundary at normal and narrow widths', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-site-modes&appearance=light');
  await page.getByRole('button', { name: 'Delivery', exact: true }).click();
  await page.locator('details[data-setting-target="app-overrides"] > summary').click();
  await page.getByRole('button', { name: /Technical Built-in/ }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  const browserSites = page.getByText('Browser sites', { exact: true });
  await expect(page.getByRole('switch', { name: 'Use browser site Mode rules' })).toBeChecked();
  await expect(page.getByText(/never reads page text or history/)).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Host for github.com' })).toHaveValue('github.com');
  await browserSites.scrollIntoViewIfNeeded();
  await page.mouse.move(0, 0);
  await expect(fixture).toHaveScreenshot('light-settings-site-modes.png');

  await page.setViewportSize({ width: 720, height: 560 });
  const testCurrentSite = page.getByRole('button', { name: 'Test current site' });
  await expect(testCurrentSite).toBeVisible();
  await testCurrentSite.scrollIntoViewIfNeeded();
  await page.mouse.move(0, 0);
  await expect(fixture).toHaveScreenshot('light-settings-site-modes-narrow.png');
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
  await expect(page.getByRole('heading', { name: 'Developing' })).toHaveCount(0);
  await expect(page.locator('.usage-analytics-section')).toHaveCount(4);
  const [workspaceBox, analyticsBox] = await Promise.all([
    page.locator('.main-dashboard-workspace').boundingBox(),
    page.locator('.usage-dashboard-content').boundingBox(),
  ]);
  expect(analyticsBox!.width).toBeGreaterThan(workspaceBox!.width * 0.9);
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-insights.png');
});

test('Voice Query provider columns stay aligned at normal and narrow widths', async ({ page }) => {
  const assertAlignedColumns = async () => {
    const table = page.locator('[role="table"][aria-label="Voice Query providers"]');
    const rows = table.locator('[role="row"]');
    await expect(rows).toHaveCount(3);
    const geometry = await rows.evaluateAll((elements) => elements.map((row) => (
      Array.from(row.children).map((cell) => {
        const box = cell.getBoundingClientRect();
        return { left: box.left, right: box.right };
      })
    )));
    for (let column = 0; column < 4; column += 1) {
      const lefts = geometry.map((row) => row[column].left);
      expect(Math.max(...lefts) - Math.min(...lefts)).toBeLessThanOrEqual(0.5);
      if (column > 0) {
        const rights = geometry.map((row) => row[column].right);
        expect(Math.max(...rights) - Math.min(...rights)).toBeLessThanOrEqual(0.5);
      }
    }
  };

  await page.goto('/visual-fixtures.html?state=insights&appearance=light');
  await assertAlignedColumns();
  await page.setViewportSize({ width: 720, height: 560 });
  await page.reload();
  await assertAlignedColumns();
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

test('meeting review keeps provenance, actions, and transcript evidence usable at normal and narrow widths', async ({ page }) => {
  await page.setViewportSize({ width: 880, height: 720 });
  await page.goto('/visual-fixtures.html?state=meetings-review&appearance=light');
  const fixture = page.locator('[data-visual-ready="true"]');
  await expect(page.getByRole('heading', { name: 'Meeting review' })).toBeVisible();
  await expect(page.getByRole('button', { name: /Summary source 1/ })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Transcript evidence' })).toBeVisible();
  await page.mouse.move(0, 0);
  await expect(fixture).toHaveScreenshot('light-meeting-review.png');

  await page.setViewportSize({ width: 720, height: 560 });
  await page.reload();
  await expect(page.getByRole('button', { name: 'Copy review' })).toBeVisible();
  await page.mouse.move(0, 0);
  await expect(fixture).toHaveScreenshot('light-meeting-review-narrow.png');
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

test('all Insights chart marks share exact hover, keyboard, click, and dismissal behavior', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=insights&appearance=light');

  const heatmap = page.locator('figure[aria-label="Words per day heatmap"]');
  const heatmapMark = heatmap.getByRole('button', { name: /760 words, 6 recordings/ });
  await heatmapMark.hover();
  await expect(heatmap.locator('.ui-day-chart-tooltip')).toContainText('760 words · 6 recordings');
  await page.getByRole('heading', { name: 'Insights' }).hover();
  await expect(heatmap.locator('.ui-day-chart-tooltip')).toContainText('Focus a day');

  const bars = page.locator('figure[aria-label="Words per day bar chart"]');
  const barMarks = bars.locator('.ui-day-chart-bar');
  await barMarks.first().focus();
  await page.keyboard.press('Tab');
  await expect(barMarks.nth(1)).toBeFocused();
  await expect(bars.locator('.ui-day-chart-tooltip')).toContainText('words');
  await page.getByRole('heading', { name: 'Insights' }).hover();
  await expect(bars.locator('.ui-day-chart-tooltip')).toContainText('words');
  await page.keyboard.press('Escape');
  await expect(bars.locator('.ui-day-chart-tooltip')).toContainText('Focus a day');

  const line = page.locator('figure[aria-label="Words-per-minute trend line"]');
  const finalPoint = line.locator('.ui-day-chart-line-targets button').last();
  await finalPoint.click();
  await expect(line.locator('.ui-day-chart-tooltip')).toContainText('WPM');
  await page.getByRole('heading', { name: 'Insights' }).click();
  await expect(line.locator('.ui-day-chart-tooltip')).toContainText('Focus a day');
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
    page.getByRole('button', { name: 'Transcribe file…', exact: true }),
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
  await expect(page.getByRole('button', { name: 'Transcribe file…' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Recent dictations' })).toBeVisible();
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-home-compact-720x560.png');
});

test('the compact 720x560 Insights view stacks analytics without horizontal clipping', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 560 });
  await page.goto('/visual-fixtures.html?state=insights&appearance=light');
  const sections = page.locator('.usage-analytics-section');
  await expect(sections).toHaveCount(4);
  expect(await sections.evaluateAll((elements) => elements.map((element) => element.dataset.analytics)))
    .toEqual(['query', 'activity', 'words', 'wpm']);
  const overflow = await page.locator('.insights-view').evaluate((element) => ({
    clientWidth: element.clientWidth,
    scrollWidth: element.scrollWidth,
  }));
  expect(overflow.scrollWidth).toBe(overflow.clientWidth);
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-insights-compact-720x560.png');
});
