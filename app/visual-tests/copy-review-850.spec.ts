import { expect, test, type Page } from '@playwright/test';

async function settings(page: Page, editor: string) {
  await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=${editor}`);
  await expect(page.locator('.settings-editor-sheet > div:first-child h2')).toBeVisible();
}

test('C1: an armed deletion blocks every modified Return native activation', async ({ page }) => {
  await settings(page, 'aliases');
  await page.getByRole('button', { name: 'Delete Murmur…', exact: true }).click();
  const confirm = page.getByRole('button', { name: 'Confirm delete Murmur…', exact: true });
  await expect(confirm).toBeFocused();
  for (const key of ['Enter', 'Shift+Enter', 'Meta+Enter', 'Control+Enter', 'Alt+Enter']) {
    await page.keyboard.press(key);
    await expect(confirm).toBeVisible();
    await expect(confirm).toBeFocused();
  }
  await page.keyboard.press('Space');
  await expect(confirm).toHaveCount(0);
});

for (const appearance of ['light', 'dark']) {
  for (const panel of ['transform', 'dictation', 'runs']) {
    test(`C2: ${appearance} ${panel} destructive dialog has no Return default`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark' });
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      await page.goto(`/visual-fixtures.html?state=copy-diagnostics&panel=${panel}&appearance=${appearance}`);
      if (panel === 'dictation') await page.getByRole('button', { name: 'Review', exact: true }).click();
      const opener = panel === 'dictation' ? page.getByRole('button', { name: 'Delete local copy…', exact: true }) : panel === 'runs' ? page.getByRole('button', { name: 'Clear performance data…', exact: true })
        : page.getByRole('button', { name: 'Delete…', exact: true });
      const attribute = panel === 'runs' ? 'data-copy-cleared-runs' : 'data-copy-deleted-capture';
      await opener.click();
      const dialog = page.locator('[role="dialog"]:not(.settings-editor-sheet)');
      await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(opener).toBeFocused();
      await expect(page.locator('html')).not.toHaveAttribute(attribute);
      await page.keyboard.press('Space');
      await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
      await expect(dialog).toHaveCount(0);
      await expect(page.locator('html')).not.toHaveAttribute(attribute);
      await page.keyboard.press('Space');
      await page.keyboard.press('Shift+Tab');
      const confirm = dialog.getByRole('button').last();
      await expect(confirm).toBeFocused();
      for (const key of ['Enter', 'Shift+Enter', 'Meta+Enter', 'Control+Enter', 'Alt+Enter']) {
        await page.keyboard.press(key);
        await expect(dialog).toBeVisible();
        await expect(page.locator('html')).not.toHaveAttribute(attribute);
      }
      await expect(dialog).toHaveScreenshot(`${appearance}-850-${panel}-confirm.png`);
      if (panel !== 'runs') {
        await page.evaluate(() => document.documentElement.setAttribute('data-copy-replace-captures', 'true'));
        if (panel === 'transform') await page.getByRole('button', { name: 'Refresh', exact: true }).evaluate((button: HTMLButtonElement) => button.click());
        await expect(page.locator('html')).toHaveAttribute('data-copy-listed-capture', 'capture-two');
        if (panel === 'transform') await expect(page.getByText('Pass 52', { exact: true })).toBeVisible();
        await expect(dialog).toContainText(panel === 'dictation' ? 'recording 41' : 'pass 51');
      }
      await page.keyboard.press('Space');
      await expect(dialog).toHaveCount(0);
      await expect(page.locator('html')).toHaveAttribute(attribute, panel === 'runs' ? 'true' : 'capture-one');
      if (panel !== 'runs') await expect(page.getByRole('heading', { name: panel === 'transform' ? 'Transform diagnostics' : 'Private dictation capture', exact: true })).toBeFocused();
    });
  }

  test(`C6: ${appearance} direct deletion retains list focus through a delayed refresh`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=copy-diagnostics&panel=transform&appearance=${appearance}`);
    await page.getByRole('button', { name: 'Delete…', exact: true }).click();
    await page.evaluate(() => document.documentElement.setAttribute('data-copy-delay-refresh', 'true'));
    await page.keyboard.press('Shift+Tab');
    await page.keyboard.press('Space');
    await expect(page.locator('html')).toHaveAttribute('data-copy-refresh-waiting', 'true');
    await expect(page.locator('[role="dialog"]:not(.settings-editor-sheet)')).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Transform diagnostics', exact: true })).toBeFocused();
    await page.evaluate(() => window.dispatchEvent(new Event('copy-release-refresh')));
    await expect(page.locator('html')).not.toHaveAttribute('data-copy-refresh-waiting');
    await expect(page.getByRole('heading', { name: 'Transform diagnostics', exact: true })).toBeFocused();
    await expect(page.locator('html')).toHaveAttribute('data-copy-deleted-capture', 'capture-one');
  });

  test(`C6: ${appearance} cancelling after a capture row disappears restores the list heading`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=copy-diagnostics&panel=transform&appearance=${appearance}`);
    await page.getByRole('button', { name: 'Delete…', exact: true }).click();
    await page.evaluate(() => document.documentElement.setAttribute('data-copy-replace-captures', 'true'));
    await page.getByRole('button', { name: 'Refresh', exact: true }).evaluate((button: HTMLButtonElement) => button.click());
    await expect(page.getByText('Pass 52', { exact: true })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('[role="dialog"]:not(.settings-editor-sheet)')).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Transform diagnostics', exact: true })).toBeFocused();
    await expect(page.locator('html')).not.toHaveAttribute('data-copy-deleted-capture');
  });

  test(`C6: ${appearance} deleting from transform review returns focus to the resulting list`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=copy-diagnostics&panel=transform&appearance=${appearance}`);
    await page.getByRole('button', { name: 'Review', exact: true }).click();
    await page.getByRole('button', { name: 'Delete capture…', exact: true }).click();
    await page.keyboard.press('Shift+Tab');
    await page.keyboard.press('Space');
    await expect(page.locator('[role="dialog"]:not(.settings-editor-sheet)')).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Transform diagnostics', exact: true })).toBeFocused();
    await expect(page.locator('html')).toHaveAttribute('data-copy-deleted-capture', 'capture-one');
  });

  test(`C3: ${appearance} invalid color Return and Done preserve the editor; valid Return commits`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark' });
    await page.goto(`/visual-fixtures.html?state=settings-copy&page=appearance&appearance=${appearance}`);
    await page.getByRole('button', { name: 'Create theme', exact: true }).click();
    const dialog = page.locator('[role="dialog"]:not(.settings-editor-sheet)');
    const accent = dialog.getByLabel('Accent', { exact: true });
    await accent.fill('not-a-color');
    await page.keyboard.press('Enter');
    await expect(dialog).toBeVisible();
    await expect(accent).toHaveAttribute('aria-invalid', 'true');
    await dialog.getByRole('button', { name: 'Done', exact: true }).click();
    await expect(dialog).toBeVisible();
    await expect(accent).toBeFocused();
    await expect(dialog).toHaveScreenshot(`${appearance}-850-invalid-accent.png`);
    await accent.fill('#123abc');
    await page.keyboard.press('Enter');
    await expect(dialog).toHaveCount(0);
    await page.getByRole('button', { name: 'Create theme', exact: true }).click();
    await expect(dialog.getByLabel('Accent', { exact: true })).toHaveValue('#123abc');
    // Pending invalid fields also block Done when Return starts elsewhere.
    await dialog.getByLabel('Background', { exact: true }).fill('invalid');
    await dialog.getByLabel('Foreground', { exact: true }).focus();
    await page.keyboard.press('Enter');
    await expect(dialog).toBeVisible();
    await expect(dialog.getByLabel('Background', { exact: true })).toBeFocused();
  });
}

test('C4: command editor owns initial focus, wraps Tab, cancels on Escape, and restores its opener', async ({ page }) => {
  await settings(page, 'commands');
  const create = page.getByRole('button', { name: 'New command…', exact: true });
  await create.click();
  const dialog = page.locator('[role="dialog"]:not(.settings-editor-sheet)');
  const first = dialog.getByRole('button', { name: 'Close voice command editor', exact: true });
  await expect(first).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(dialog.getByRole('button', { name: 'Save command', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(first).toBeFocused();
  await dialog.getByLabel('Voice command type', { exact: true }).selectOption('snippet');
  await dialog.getByLabel('Voice command content', { exact: true }).fill('line one');
  await page.keyboard.press('Shift+Enter');
  await expect(dialog.getByLabel('Voice command content', { exact: true })).toHaveValue('line one\n');
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(create).toBeFocused();
});

test('C4: Knowledge confirmation traps focus and restores the overflow trigger without test focus calls', async ({ page }) => {
  await settings(page, 'knowledge');
  const opener = page.getByRole('button', { name: 'More knowledge actions', exact: true });
  await opener.click();
  await page.getByRole('menuitem', { name: 'Delete all…', exact: true }).click();
  const dialog = page.locator('[role="dialog"]:not(.settings-editor-sheet)');
  const input = dialog.getByLabel('Type DELETE to confirm', { exact: true });
  await expect(input).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(input).toBeFocused();
  await input.fill('DELETE');
  await page.keyboard.press('Shift+Tab');
  await expect(dialog.getByRole('button', { name: 'Delete everything…', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(input).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(opener).toBeFocused();
});

for (const appearance of ['light', 'dark']) {
  for (const permission of ['unknown', 'granted', 'denied', 'loading', 'restricted', 'unsupported']) {
    test(`C5: ${appearance} ${permission} permission steps have at most one primary and only enabled Continue is a default`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark' });
      await page.goto(`/visual-fixtures.html?state=copy-onboarding&permission=${permission}&appearance=${appearance}`);
      await page.getByRole('button', { name: 'Get started', exact: true }).click();
      for (const title of ['Microphone access', 'Accessibility access', 'System audio access', 'Calendar access']) {
        await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
        const next = page.getByRole('button', { name: 'Continue', exact: true });
        const primary = page.locator('button').filter({ hasNotText: /^$/ });
        const assertPrimary = async () => {
          const styles = await primary.evaluateAll(buttons => buttons.map(button => ({
            text: button.textContent?.trim(), primary: button.className.includes('text-on-primary'),
            background: getComputedStyle(button).backgroundImage,
          })));
          expect(styles.filter(button => button.primary).length).toBeLessThanOrEqual(1);
          if (permission === 'granted') expect(styles.filter(button => button.primary)).toHaveLength(1);
          const continueStyle = styles.find(button => button.text === 'Continue')!;
          expect(continueStyle.primary).toBe(permission === 'granted');
          expect(continueStyle.background === 'none').toBe(permission !== 'granted');
        };
        await assertPrimary();
        if (permission === 'unknown') await expect(page.locator('.main-first-run-onboarding')).toHaveScreenshot(`${appearance}-850-${title.split(' ')[0].toLowerCase()}-permission.png`);
        if (permission === 'loading' && ['System audio access', 'Calendar access'].includes(title)) {
          await page.getByRole('button', { name: title === 'Calendar access' ? 'Allow Calendar access' : 'Allow system audio access', exact: true }).click();
          await expect(page.getByRole('button', { name: 'Waiting for macOS…', exact: true })).toBeDisabled();
          await assertPrimary();
        }
        if (permission === 'loading' && ['Microphone access', 'Accessibility access'].includes(title)) {
          await page.getByRole('button', { name: title === 'Microphone access' ? 'Allow microphone access…' : 'Grant accessibility access…', exact: true }).click();
          await expect(page.getByText(title === 'Microphone access'
            ? 'Waiting for your answer in the macOS dialog…'
            : 'Murmur is listed and enabled, but still not detected? Reset the permission', { exact: true })).toBeVisible();
          await assertPrimary();
        }
        const back = page.getByRole('button', { name: 'Go back to the previous setup step' });
        await back.focus();
        await page.keyboard.press('Enter');
        if (permission !== 'granted') {
          await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
          await expect(next).toBeDisabled();
          await page.getByRole('button', { name: /^Skip / }).click();
        }
      }
      await expect(page.getByRole('heading', { name: 'Transcription model', exact: true })).toBeVisible();
    });
  }
}
