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

test('one filter menu keeps its choice selected while hovered and names it on the trigger', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/visual-fixtures.html?state=idle&appearance=dark');
  const trigger = page.getByRole('button', { name: /^Filter transcripts/ });
  await expect(trigger).toHaveText('Filter');
  await trigger.click();
  const spoken = page.getByRole('menuitemradio', { name: 'Spoken', exact: true });
  await spoken.click();
  await spoken.hover();
  await expect(spoken).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByRole('menuitemradio', { name: 'Everything', exact: true })).toHaveAttribute('aria-checked', 'false');
  await expect(trigger).toHaveText('Spoken');
  await expect(page.locator('.history-filtered-note')).toContainText('Showing');
});

test('date filters compose with source filters and Markdown copy', async ({ page, context, baseURL }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: baseURL });
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  const cards = page.locator('.home-history .transcript-card');
  await expect(cards).toHaveCount(16);
  await page.getByRole('button', { name: /^Filter transcripts/ }).click();
  await page.getByRole('menuitemradio', { name: 'Today', exact: true }).click();
  await expect(cards).toHaveCount(3);
  await page.getByRole('menuitemradio', { name: 'Files', exact: true }).click();
  await expect(cards).toHaveCount(1);
  await page.keyboard.press('Escape');
  await cards.first().getByRole('button', { name: 'More transcript actions' }).click();
  await page.getByRole('menuitem', { name: 'Copy as Markdown', exact: true }).click();
  await expect(page.getByText('Copied transcript as Markdown.', { exact: true })).toBeVisible();
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toContain('# Murmur');
  expect(copied).toContain('Imported audio uses the same spacing rhythm');
  expect(copied).not.toContain('The compact transcript keeps its metadata');
  await page.getByRole('searchbox', { name: 'Search transcripts' }).fill('no-matching-fixture');
  await page.locator('.history-filtered-note').getByRole('button', { name: 'Show all' }).click();
  await expect(cards).toHaveCount(16);
  await expect(page.getByRole('button', { name: 'Filter transcripts', exact: true })).toHaveText('Filter');
  await expect(page.locator('.history-filtered-note')).toHaveCount(0);
});

test('pins a transcript and filters it through the real history fixture', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  const cards = page.locator('.home-history .transcript-card');
  const target = cards.first();
  const targetText = await target.locator('.transcript-text').innerText();
  await target.getByRole('button', { name: 'More transcript actions' }).click();
  await page.getByRole('menuitem', { name: 'Pin transcript', exact: true }).click();
  await expect(page.getByText('Transcript pinned.', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: /^Filter transcripts/ }).click();
  const pinnedOnly = page.getByRole('menuitemcheckbox', { name: 'Pinned only', exact: true });
  await pinnedOnly.click();
  await expect(pinnedOnly).toHaveAttribute('aria-checked', 'true');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Filter transcripts: Pinned', exact: true })).toBeVisible();
  await expect(cards).toHaveCount(1);
  await expect(cards.first().locator('.transcript-text')).toHaveText(targetText);
  await cards.first().getByRole('button', { name: 'More transcript actions' }).click();
  await page.getByRole('menuitem', { name: 'Unpin transcript', exact: true }).click();
  await expect(page.getByText('No matching transcripts', { exact: true })).toBeVisible();
});

test('copied transcripts keep their geometry and actions reachable', async ({ page, context, baseURL }) => {
  await context.grantPermissions(['clipboard-write'], { origin: baseURL });
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
  await middleCard.scrollIntoViewIfNeeded();
  const before = await middleCard.boundingBox();
  const shadowBefore = await middleCard.evaluate((element) => getComputedStyle(element).boxShadow);

  await expect(middleCard).toHaveAttribute('data-day-end', 'false');
  await middleCard.click();
  await expect(middleCard).toHaveAttribute('data-copied', 'true');
  await expect(feedback).toHaveText('Copied');
  await expect(copyAction).toHaveText('Copied');
  expect(await middleCard.boundingBox()).toEqual(before);
  expect(await middleCard.evaluate((element) => getComputedStyle(element).boxShadow)).toBe(shadowBefore);
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
  await page.setViewportSize({ width: 1120, height: 820 });
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
  expect(mainBox?.width).toBeGreaterThan(650);
  expect(railBox?.width).toBe(212);
  expect(railBox?.y).toBe(mainBox?.y);
  await expect(page.getByText('This month', { exact: true })).toBeVisible();
  await expect(page.getByText('Voice profile', { exact: true })).toHaveCount(0);
  await expect(page.getByTestId('main-status-chip')).toHaveCount(0);
  await expect(page.locator('.home-history .history-date-label')).toHaveCount(14);
});

for (const size of [{ width: 1120, height: 820 }, { width: 880, height: 720 }] as const) {
  test(`home talk card and history toolbar each fit on one line at ${size.width}x${size.height}`, async ({ page }) => {
    await page.setViewportSize(size);
    await page.goto('/visual-fixtures.html?state=idle&appearance=light');

    const talk = page.locator('.home-talk');
    await expect(talk).toContainText('Click to start talking');
    const talkBox = await talk.boundingBox();
    expect(talkBox?.height).toBeLessThanOrEqual(56);
    for (const line of [page.locator('.home-talk-title strong'), page.locator('.home-talk-hint')]) {
      expect(await line.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    }

    const toolbar = page.locator('.home-history .history-toolbar');
    const centers = await toolbar.evaluate((element) => Array.from(element.children).map((child) => {
      const box = child.getBoundingClientRect();
      return box.top + box.height / 2;
    }));
    expect(centers.length).toBe(4);
    expect(Math.max(...centers) - Math.min(...centers)).toBeLessThanOrEqual(1);
  });
}

for (const size of [{ width: 880, height: 720 }, { width: 800, height: 600 }, { width: 761, height: 600 }] as const) {
  test(`combined history filters keep every toolbar action reachable at ${size.width}x${size.height}`, async ({ page }) => {
    await page.setViewportSize(size);
    await page.goto('/visual-fixtures.html?state=idle&appearance=light');

    await page.getByRole('button', { name: 'Filter transcripts', exact: true }).click();
    await page.getByRole('menuitemradio', { name: 'Spoken', exact: true }).click();
    await page.getByRole('menuitemradio', { name: 'Past 30 days', exact: true }).click();
    await page.getByRole('menuitemcheckbox', { name: 'Pinned only', exact: true }).click();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('button', { name: 'Filter transcripts: 3 filters, Spoken · Past 30 days · Pinned' })).toHaveText('3 filters');

    const column = await page.locator('.home-history').boundingBox();
    const more = await page.getByRole('button', { name: 'More history actions' }).boundingBox();
    expect(more!.x + more!.width).toBeLessThanOrEqual(column!.x + column!.width + 0.5);
    const title = page.getByRole('heading', { name: 'Your dictations' });
    expect(await title.evaluate((element) => element.getBoundingClientRect().height)).toBeLessThanOrEqual(24);
  });
}

for (const state of ['recording', 'update-recovering', 'settings'] as const) {
  test(`${state} title-bar controls share the native traffic-light centerline`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=${state}&appearance=light`);
    // Home shows status in its talk card; the title-bar chip appears on the other pages.
    if (state !== 'settings') await page.getByRole('button', { name: 'Insights', exact: true }).click();

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
  await expect(update).toHaveAccessibleName('Download Update. Murmur v0.27.1 is available');
  await expect(record).toHaveAccessibleName('Reconnecting your microphone…');
  await expect(record).toBeDisabled();

  const geometry = await Promise.all([
    header.boundingBox(),
    update.boundingBox(),
    record.boundingBox(),
  ]);
  const [headerBox, updateBox, recordBox] = geometry;

  expect(updateBox?.width).toBeLessThanOrEqual(28);
  expect(updateBox?.height).toBeLessThanOrEqual(28);
  expect(recordBox?.height).toBeGreaterThanOrEqual(44);
  expect(recordBox?.height).toBeLessThanOrEqual(56);
  expect(headerBox?.height).toBe(42);
});

for (const notes of ['short', 'long'] as const) {
  test(`update dialog gives ${notes} release notes the main reading area`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=update-dialog-${notes}&appearance=light`);

    const fixture = page.locator('[data-visual-ready="true"]');
    const dialog = page.getByRole('dialog', { name: 'Update Available' });
    const notesArea = dialog.locator('.overflow-y-auto');
    const actions = dialog.getByRole('button', { name: 'Download Update' }).locator('..');
    const [notesBox, actionsBox] = await Promise.all([
      notesArea.boundingBox(),
      actions.boundingBox(),
    ]);

    expect(notesBox?.height).toBeGreaterThan(actionsBox?.height ?? 0);
    await expect(dialog.getByRole('button', { name: 'Download Update' })).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Skip This Version' })).toHaveCount(0);
    await expect(dialog.getByRole('button', { name: 'Later' })).toBeVisible();
    await expect(fixture).toHaveScreenshot(`light-update-dialog-${notes}.png`);
  });
}

test('update dialog keeps every action visible in a smaller window', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 560 });
  await page.goto('/visual-fixtures.html?state=update-dialog-long&appearance=light');

  const dialog = page.getByRole('dialog', { name: 'Update Available' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Download Update' })).toBeInViewport();
  await expect(dialog.getByRole('button', { name: 'Skip This Version' })).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: 'Later' })).toBeInViewport();
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-update-dialog-long-small.png');
});

test('customization hub stays legible and restores focus at native and narrow widths', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings&appearance=light');

  const fixture = page.locator('[data-visual-ready="true"]');
  const customizationList = page.getByRole('list', { name: 'Customization destinations' });
  const voiceCommands = customizationList.getByRole('button').filter({ hasText: 'Voice Commands' });
  await expect(page.getByRole('heading', { name: 'Customize Murmur' })).toBeVisible();
  await expect(customizationList.getByRole('button')).toHaveCount(5);
  await expect(fixture).toHaveScreenshot('light-settings-customization-hub.png');

  await voiceCommands.click();
  await expect(page.getByRole('heading', { name: 'Voice Commands', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Back to Customize' }).click();
  await expect(voiceCommands).toBeFocused();

  await page.setViewportSize({ width: 720, height: 560 });
  await expect(page.getByRole('heading', { name: 'Customize Murmur' })).toBeVisible();
  await expect(fixture).toHaveScreenshot('light-settings-customization-hub-narrow.png');
});

test('synthetic meeting suggestion overlay keeps its prompt and actions visible', async ({ page }) => {
  await page.setViewportSize({ width: 520, height: 260 });
  await page.goto('/visual-fixtures.html?state=overlay-meeting-suggestion&appearance=dark');

  const fixture = page.locator('[data-fixture-synthetic="meeting-suggestion"]');
  await expect(page.getByText('Synthetic calendar: Product review started. Start Notetaker?')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Accept' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Dismiss' })).toBeVisible();
  await expect(fixture).toHaveScreenshot('synthetic-meeting-suggestion-overlay.png');
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
  await expect(page.getByRole('button', { name: 'Microphone input' })).toContainText(
    'Follow macOS Default — MacBook Pro Microphone',
  );
  await expect(page.getByText(/Docking, undocking, or changing the system input/)).toBeVisible();
  await expect(fixture).toHaveScreenshot('light-settings-recording-auto-microphone.png');
});

test('recording settings show Smart Auto inclusion and selection status', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-smart-auto&appearance=light');
  await page.getByRole('button', { name: 'Recording', exact: true }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  const picker = page.getByRole('button', { name: 'Microphone input' });
  await expect(picker).toContainText('Smart Auto · Available: MacBook Pro Microphone');
  const disclosure = page.getByRole('button', { name: 'Smart Auto microphones' });
  const submenu = page.getByRole('group', { name: 'Smart Auto microphone inclusion' });
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false');
  await expect(submenu).toBeHidden();
  await expect(page.getByText('Smart Auto will use MacBook Pro Microphone.')).toBeVisible();
  await expect(page.getByText(/Why: your preferred included microphone/)).toBeVisible();
  await expect(page.getByText(/Live input from MacBook Pro Microphone/)).toBeVisible();
  await expect(page.getByRole('button', { name: /Verify signal/ })).toHaveCount(0);
  await expect(fixture).toHaveScreenshot('light-settings-recording-smart-auto.png');

  await picker.click();
  const pickerDialog = page.getByRole('dialog', { name: 'Choose microphone mode' });
  await expect(pickerDialog.getByRole('radio', { name: /Smart Auto/ })).toBeChecked();
  await expect(pickerDialog.getByRole('checkbox')).toHaveCount(0);
  await expect(pickerDialog.getByRole('listbox')).toHaveCount(0);
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false');
  await expect(fixture).toHaveScreenshot('light-settings-recording-smart-auto-picker.png');
  await picker.click();
  await expect(pickerDialog).toHaveCount(0);
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false');

  await disclosure.click();
  await expect(disclosure).toHaveAttribute('aria-expanded', 'true');
  await expect(submenu.getByRole('checkbox', { name: /MacBook Pro Microphone/ })).toBeChecked();
  await expect(submenu.getByRole('checkbox', { name: /Anker USB Microphone/ })).toBeChecked();
  await expect(submenu.getByRole('checkbox', { name: /Desk Microphone/ })).toBeChecked();
  await expect(submenu.getByText('Not connected')).toBeVisible();
  await expect(submenu.getByText('Included', { exact: true })).toHaveCount(3);
  await expect(submenu.getByText('Candidate')).toBeVisible();
  await expect(submenu.getByRole('button', { name: 'Prefer MacBook Pro Microphone for Smart Auto' })).toHaveText('Preferred');
  await expect(submenu.getByRole('button', { name: 'Prefer Anker USB Microphone for Smart Auto' })).toHaveText('Prefer');
  await expect(submenu.getByRole('switch', { name: 'Check included microphones in the background' })).toBeChecked();
  await expect(fixture).toHaveScreenshot('light-settings-recording-smart-auto-status.png');

  await picker.click();
  await expect(disclosure).toHaveAttribute('aria-expanded', 'true');
  await picker.click();
  await expect(pickerDialog).toHaveCount(0);
  await expect(disclosure).toHaveAttribute('aria-expanded', 'true');

  await picker.click();
  const smartAutoMode = pickerDialog.getByRole('radio', { name: /Smart Auto/ });
  await smartAutoMode.focus();
  await smartAutoMode.press('ArrowDown');
  await expect(picker).toContainText('Follow macOS Default');
  await expect(pickerDialog).toHaveCount(0);
  await expect(picker).toBeFocused();
});

test('recording settings explain Smart Auto with no eligible microphones', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-smart-auto-empty&appearance=light');
  await page.getByRole('button', { name: 'Recording', exact: true }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  const disclosure = page.getByRole('button', { name: 'Smart Auto microphones' });
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false');
  await disclosure.click();
  const submenu = page.getByRole('group', { name: 'Smart Auto microphone inclusion' });
  await expect(submenu.getByText('Excluded', { exact: true })).toHaveCount(3);
  await expect(submenu.getByText('No microphones are included. Include at least one available input for Smart Auto.')).toBeVisible();
  await expect(page.getByText('Smart Auto is not ready.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry background checks' })).toHaveCount(0);
  await expect(fixture).toHaveScreenshot('light-settings-recording-smart-auto-empty.png');
});

test('recording settings keep Smart Auto ready when background checks are off', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-smart-auto-no-probes&appearance=light');
  await page.getByRole('button', { name: 'Recording', exact: true }).click();

  const fixture = page.locator('[data-visual-ready="true"]');
  const disclosure = page.getByRole('button', { name: 'Smart Auto microphones' });
  const submenu = page.getByRole('group', { name: 'Smart Auto microphone inclusion' });
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByText('Smart Auto will use MacBook Pro Microphone.')).toBeVisible();
  await disclosure.click();
  await expect(submenu.getByRole('checkbox', { name: /MacBook Pro Microphone/ })).toBeChecked();
  await expect(submenu.getByRole('checkbox', { name: /Anker USB Microphone/ })).toBeChecked();
  await expect(submenu.getByRole('switch', { name: 'Check included microphones in the background' })).not.toBeChecked();
  await expect(submenu.getByText(/When off, Murmur chooses an included available microphone/)).toBeVisible();
  await expect(fixture).toHaveScreenshot('light-settings-recording-smart-auto-no-probes.png');

  await disclosure.click();
  await expect(disclosure).toHaveAttribute('aria-expanded', 'false');
  await expect(submenu).toBeHidden();
  await expect(page.getByText('Smart Auto will use MacBook Pro Microphone.')).toBeVisible();
});

test('settings rows keep the shared spacing contract and aligned controls', async ({ page }) => {
  // Page-wide gaps are measured by settings-rhythm.spec.ts; this pins the row
  // geometry that rhythm relies on: one row padding, one inset, and switches
  // and titles that line up whether a toggle is a row or sits inside a group.
  await page.goto('/visual-fixtures.html?state=settings&appearance=light');
  const rowGeometry = (target: string) => page.locator(`[data-setting-target="${target}"]`).evaluate((row) => {
    const style = getComputedStyle(row);
    const box = row.getBoundingClientRect();
    const title = row.querySelector('p')!.getBoundingClientRect();
    const toggle = row.querySelector('[role="switch"]')!.getBoundingClientRect();
    return {
      left: box.left,
      right: box.right,
      padding: [style.paddingTop, style.paddingBottom],
      titleLeft: title.left,
      switchRight: toggle.right,
    };
  });
  const expectAligned = async (targets: [string, string]) => {
    const [first, second] = await Promise.all(targets.map(rowGeometry));
    expect(first.padding).toEqual(['12px', '12px']);
    expect(second.padding).toEqual(['12px', '12px']);
    expect(second.left).toBe(first.left);
    expect(second.right).toBe(first.right);
    expect(second.titleLeft).toBe(first.titleLeft);
    expect(second.switchRight).toBe(first.switchRight);
  };

  await page.getByRole('button', { name: 'Recording', exact: true }).click();
  await page.getByRole('button', { name: 'Double-Tap', exact: true }).click();
  await expectAligned(['hotkey-feedback', 'sound-cues']);

  const rail = page.locator('[data-setting-target="sound-cues"] .settings-dependent-branch-content');
  const railInsets = await rail.evaluate((element) => {
    const style = getComputedStyle(element, '::before');
    return { top: Number.parseFloat(style.top), bottom: Number.parseFloat(style.bottom) };
  });
  expect(railInsets.top).toBeGreaterThan(0);
  expect(railInsets.bottom).toBeGreaterThan(0);
  expect(Math.abs(railInsets.top - railInsets.bottom)).toBeLessThan(0.1);

  await page.getByRole('button', { name: 'Meetings', exact: true }).click();
  await expectAligned(['meeting-audio', 'meeting-speakers']);

  await page.getByRole('button', { name: 'Delivery', exact: true }).click();
  await expectAligned(['auto-paste', 'file-output']);
});

test('browser-site Mode rules disclose their exact privacy boundary at normal and narrow widths', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-site-modes&appearance=light');
  await page.getByRole('button', { name: 'Modes', exact: true }).click();
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
  const sectionBottoms = await page.locator('.usage-analytics-section').evaluateAll((sections) => (
    sections.map((section) => section.getBoundingClientRect().bottom)
  ));
  expect(sectionBottoms[0]).toBe(sectionBottoms[1]);
  expect(sectionBottoms[2]).toBe(sectionBottoms[3]);
  const [workspaceBox, analyticsBox] = await Promise.all([
    page.locator('.main-dashboard-workspace').boundingBox(),
    page.locator('.usage-dashboard-content').boundingBox(),
  ]);
  expect(analyticsBox!.width).toBeGreaterThan(workspaceBox!.width * 0.9);
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-insights.png');
});

test('populated Insights keeps all tiles and charts reachable in the default window', async ({ page }) => {
  await page.setViewportSize({ width: 1120, height: 820 });
  await page.goto('/visual-fixtures.html?state=insights&appearance=light');

  const view = page.locator('.insights-view');
  const fit = await view.evaluate((element) => {
    const viewBounds = element.getBoundingClientRect();
    const dateLabels = Array.from(element.querySelectorAll('.ui-day-chart-axis span'));
    const heatmapCell = element.querySelector('.ui-day-chart-heatmap button');
    const sections = Array.from(element.querySelectorAll('.usage-analytics-section'));
    return {
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      datesContained: dateLabels.every((label) => {
        const bounds = label.getBoundingClientRect();
        return bounds.left >= viewBounds.left && bounds.right <= viewBounds.right;
      }),
      heatmapCellWidth: heatmapCell?.getBoundingClientRect().width ?? Number.POSITIVE_INFINITY,
      sectionBottoms: sections.map((section) => section.getBoundingClientRect().bottom),
    };
  });

  await expect(view).toHaveCSS('overflow-y', 'auto');
  expect(fit.scrollWidth).toBe(fit.clientWidth);
  expect(fit.datesContained).toBe(true);
  expect(fit.heatmapCellWidth).toBeLessThanOrEqual(32);
  expect(fit.sectionBottoms[0]).toBe(fit.sectionBottoms[1]);
  expect(fit.sectionBottoms[2]).toBe(fit.sectionBottoms[3]);
  const tiles = view.locator('[aria-label="Usage totals"] > .ui-dashboard-stat');
  await expect(tiles).toHaveCount(10);
  for (const item of [...await tiles.all(), ...await view.locator('.usage-analytics-section').all()]) {
    await item.scrollIntoViewIfNeeded();
    await expect(item).toBeInViewport({ ratio: 1 });
  }
  await expect(page.locator('[data-query-note="failures"]')).toContainText('provider not authenticated 1');
  await expect(page.locator('.usage-analytics-section').first()).toHaveCSS('border-top-width', '1px');
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

test('meeting captions explain segment timing and keep copy and export reachable', async ({ page }) => {
  for (const [appearance, width, height, format] of [
    ['light', 880, 720, 'srt'],
    ['dark', 720, 560, 'vtt'],
  ] as const) {
    await page.setViewportSize({ width, height });
    await page.goto(`/visual-fixtures.html?state=meetings-review&appearance=${appearance}`);
    await page.getByRole('combobox', { name: 'Meeting review export format' }).selectOption(format);
    await expect(page.getByRole('button', { name: 'Copy captions' })).toBeVisible();
    await expect(page.getByText('One caption per recorded speech segment', { exact: false })).toBeVisible();
    const exportButton = page.getByRole('button', { name: 'Export…', exact: true });
    await expect(exportButton).toBeVisible();
    const bounds = await exportButton.boundingBox();
    expect(bounds && bounds.x + bounds.width).toBeLessThanOrEqual(width);
    await page.mouse.move(0, 0);
    await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot(`${appearance}-meeting-captions.png`);
  }
});

test('meeting calendar picker requires selection and keeps denied access usable', async ({ page }) => {
  await page.setViewportSize({ width: 880, height: 720 });
  await page.goto('/visual-fixtures.html?state=meetings-review&appearance=light');
  await page.getByRole('button', { name: 'Name from calendar', exact: true }).click();
  await expect(page.getByRole('radio')).toHaveCount(2);
  await expect(page.getByRole('button', { name: 'Apply event' })).toBeDisabled();
  await page.getByRole('radio', { name: /Orion planning/ }).focus();
  await page.keyboard.press('Space');
  await expect(page.getByRole('button', { name: 'Apply event' })).toBeEnabled();
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-meeting-calendar-picker.png');

  await page.goto('/visual-fixtures.html?state=meetings-review&appearance=light&calendar=denied');
  await page.getByRole('button', { name: 'Name from calendar', exact: true }).click();
  await expect(page.getByLabel('Meeting title', { exact: true })).toBeEditable();
  await expect(page.getByRole('button', { name: 'Open Calendar Settings' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reset Calendar Access' })).toBeVisible();
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-meeting-calendar-denied.png');
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
  const relativeGeometry = (boxes: typeof before) => boxes.map((box) => ({
    x: box && boxes[2] ? box.x - boxes[2].x : null,
    y: box && boxes[2] ? box.y - boxes[2].y : null,
    width: box?.width ?? null,
    height: box?.height ?? null,
  }));
  expect(relativeGeometry(after)).toEqual(relativeGeometry(before));

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
  const disabled = page.getByTestId('home-record-button');
  await expect(disabled).toBeDisabled();
  await expect(disabled).toHaveCSS('cursor', 'not-allowed');
  await expect(disabled).toHaveCSS('opacity', '1');
});

test('the compact 720x560 home keeps actions and history reachable', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 560 });
  await page.goto('/visual-fixtures.html?state=idle&appearance=light');
  await expect(page.getByRole('button', { name: 'Click to start talking' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'More history actions' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Your dictations' })).toBeVisible();
  await expect(page.locator('.discovery-checklist-items li')).toHaveCount(7);
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-home-compact-720x560.png');
});

test('discovery checklist and contextual hint remain readable together', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=discovery-hint&appearance=light');
  await expect(page.getByRole('heading', { name: 'Try what is ready when you are' })).toBeVisible();
  await expect(page.getByText('Use a Mode for this site', { exact: true })).toBeVisible();
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-discovery-hint.png');
});

test('checklist feature names and pitches stay readable beside the Insights rail', async ({ page }) => {
  for (const [width, height] of [[1120, 820], [880, 720], [720, 560]]) {
    await page.setViewportSize({ width, height });
    await page.goto('/visual-fixtures.html?state=idle&appearance=light');
    const labels = page.locator('.discovery-checklist-items strong, .discovery-checklist-items small');
    await expect(labels).toHaveCount(14);
    const clipped = await labels.evaluateAll((elements) => elements
      .filter((element) => element.scrollWidth > element.clientWidth + 1
        || element.scrollHeight > element.clientHeight + 1)
      .map((element) => element.textContent));
    expect(clipped, `Checklist text at ${width}x${height}`).toEqual([]);
  }
});

test('Transform practice shows the real hold-flow instructions', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-transform-practice&appearance=light');
  await expect(page.getByRole('heading', { name: 'Practice on sample text' })).toBeVisible();
  await page.getByRole('button', { name: 'Select sample text' }).click();
  await expect(page.locator('.transform-practice [role="status"]')).toContainText('make this shorter');
  await expect(page.locator('[data-visual-ready="true"]')).toHaveScreenshot('light-settings-transform-practice.png');
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
