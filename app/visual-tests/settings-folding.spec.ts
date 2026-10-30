import { expect, test, type Page } from '@playwright/test';

const panes = ['general', 'recording', 'text', 'delivery', 'meetings', 'ai-transcription', 'ai-query', 'ai-transform'] as const;
async function open(page: Page, pane: string, appearance = 'light', motion: 'reduce' | 'no-preference' = 'reduce') {
  await page.emulateMedia({ reducedMotion: motion, colorScheme: appearance as 'light' | 'dark' });
  await page.goto(`/visual-fixtures.html?state=settings-folding&page=${pane}&appearance=${appearance}`);
  await expect(page.locator('[data-visual-ready=true]')).toBeVisible();
}
const advanced = (page: Page) => page.getByRole('button', { name: /^Advanced/ });

for (const appearance of ['light', 'dark']) {
  for (const pane of panes) {
    test(`${appearance} ${pane} folded and unfolded`, async ({ page }) => {
      await open(page, pane, appearance);
      await expect(advanced(page)).toHaveAttribute('aria-expanded', 'false');
      await expect(page).toHaveScreenshot(`${appearance}-${pane}-folded.png`);
      await advanced(page).click();
      // Exercise nested parent folds as well as Advanced. No automatic enabling
      // in production: these are explicit user interactions in the fixture.
      if (pane === 'recording') {
        await page.getByRole('switch', { name: 'Warm microphone standby', exact: true }).check();
      }
      if (pane === 'text') {
        await page.getByRole('switch', { name: 'Developer terms', exact: true }).check();
        await page.getByRole('switch', { name: 'Transcript cleanup', exact: true }).check();
      }
      await advanced(page).evaluate(el => el.scrollIntoView({ block: 'start' }));
      await expect(page).toHaveScreenshot(`${appearance}-${pane}-unfolded.png`);
    });
  }
  test(`${appearance} browser site rows fold inside their card`, async ({ page }) => {
    await open(page, 'modes', appearance);
    const toggle = page.getByRole('switch', { name: 'Use browser site mode rules' });
    await toggle.scrollIntoViewIfNeeded();
    await expect(page.getByRole('textbox', { name: 'Site rule host', exact: true })).toHaveCount(0);
    await expect(page).toHaveScreenshot(`${appearance}-modes-folded.png`);
    await toggle.check();
    await expect(page.getByRole('textbox', { name: 'Site rule host', exact: true })).toBeVisible();
    await expect(page).toHaveScreenshot(`${appearance}-modes-unfolded.png`);
  });
  test(`${appearance} appearance editor remains a sheet`, async ({ page }) => {
    await open(page, 'appearance', appearance);
    await expect(page).toHaveScreenshot(`${appearance}-appearance-folded.png`);
    await page.getByRole('button', { name: 'Create theme', exact: true }).click();
    await expect(page.getByRole('dialog', { name: 'Create theme' })).toBeVisible();
    await expect(page).toHaveScreenshot(`${appearance}-appearance-unfolded.png`);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('button', { name: 'Create theme', exact: true })).toBeFocused();
  });
  for (const [tab, label] of [['vocabulary', 'Vocabulary'], ['aliases', 'Aliases'], ['knowledge', 'Knowledge'], ['commands', 'Voice commands'], ['scan', 'Manage project scan'], ['transforms', 'Manage'], ['app-overrides', 'Manage app overrides']] as const) {
    test(`${appearance} ${tab} editor sheet retains owner`, async ({ page }) => {
      await open(page, tab === 'transforms' ? 'ai-transform' : tab === 'app-overrides' ? 'delivery' : 'text', appearance);
      if (tab === 'app-overrides') await advanced(page).click();
      if (tab === 'scan') {
        await advanced(page).click();
        await page.getByRole('switch', { name: 'Developer terms', exact: true }).check();
      }
      const opener = page.locator('main').getByRole('button', { name: new RegExp(`^${label}`) }).first();
      await opener.scrollIntoViewIfNeeded();
      await expect(page).toHaveScreenshot(`${appearance}-${tab}-sheet-closed.png`);
      await opener.click();
      const dialog = page.getByRole('dialog');
      await expect(dialog).toHaveCount(1);
      await expect(dialog).toBeVisible();
      await expect(page.locator('.settings-sidebar').locator('..')).toHaveAttribute('inert', '');
      for (let i = 0; i < 4; i++) {
        await page.keyboard.press('Tab');
        expect(await dialog.evaluate(el => el.contains(document.activeElement))).toBe(true);
      }
      await expect(page).toHaveScreenshot(`${appearance}-${tab}-sheet-open.png`);
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(opener).toBeFocused();
    });
  }
}

test('folds use the requested timing, preserve values, and immediately exclude closing rows from AX/Tab', async ({ page }) => {
  await open(page, 'text', 'light', 'no-preference');
  const cleanup = page.getByRole('switch', { name: 'Transcript cleanup', exact: true });
  await cleanup.check();
  const child = page.getByRole('switch', { name: 'Remove filler words', exact: true });
  await child.uncheck();
  const branch = page.locator('[data-setting-target=cleanup] + .settings-dependent-branch');
  await expect(branch).toHaveCSS('transition-duration', '0.3s, 0.3s, 0s');
  await expect(child).toBeVisible();
  await child.focus();
  // Keyboard focus can also leave via an external config update; the component
  // test covers that path. Here the real pointer owner closes the branch.
  await cleanup.uncheck();
  await expect(branch).toHaveAttribute('inert', '');
  await expect(branch).toHaveAttribute('aria-hidden', 'true');
  await expect(branch).toHaveCSS('transition-duration', '0.45s, 0.45s, 0s');
  await expect(page.getByRole('switch', { name: 'Remove filler words', exact: true })).toHaveCount(0);
  await expect(branch).toHaveCSS('opacity', '0.4');
  await expect(branch).toHaveCSS('grid-template-rows', '0px');
  await cleanup.check();
  await expect(child).not.toBeChecked();
  const childRow = branch.locator('[data-setting-target=cleanup-filler]');
  await expect(childRow).toHaveCSS('border-top-width', '1px');
  await expect(childRow).toHaveCSS('padding', '12px 16px');
  await expect(childRow.locator('.settings-row-title')).toHaveCSS('font-size', '14px');
  await expect(childRow.locator('.settings-row-title')).toHaveCSS('line-height', '20px');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await cleanup.uncheck();
  expect(parseFloat(await branch.evaluate(el => getComputedStyle(el).transitionDuration))).toBeLessThan(.001);
  await expect(branch).toHaveCSS('grid-template-rows', '0px');
});

test('Advanced remembers only known pane UI state across navigation and relaunch', async ({ page }) => {
  await open(page, 'general');
  await advanced(page).click();
  await page.getByRole('button', { name: 'Recording', exact: true }).click();
  await expect(advanced(page)).toHaveAttribute('aria-expanded', 'false');
  await page.reload();
  await expect(advanced(page)).toHaveAttribute('aria-expanded', 'true'); // URL opens General again
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('murmur.settings-disclosure.v1')!));
  expect(stored).toEqual({ version: 1, panes: { general: true } });
  await page.evaluate(() => localStorage.setItem('murmur.settings-disclosure.v1', '{broken'));
  await page.reload();
  await expect(advanced(page)).toHaveAttribute('aria-expanded', 'false');
});

for (const [query, parent, child] of [
  ['Project scan', 'developer-terms', 'project-scan'],
  ['Remove filler words', 'cleanup', 'cleanup-filler'],
  ['Paste delay', 'auto-paste', 'paste-delay'],
  ['Query shortcut', 'voice-query-shortcut', 'query-key'],
  ['Transform hold key', 'transform-shortcut', 'transform-key'],
  ['Browser site rules', 'browser-sites', 'browser-site-rules'],
] as const) {
  test(`search for ${query} reveals its off parent without enabling anything`, async ({ page }) => {
    await open(page, 'text', 'light', 'no-preference');
    // Defaults have query/transform/site/cleanup/developer parents off. Auto-paste
    // defaults on, so explicitly turn it off first as a real user would.
    if (parent === 'auto-paste') {
      await page.getByRole('button', { name: 'Delivery', exact: true }).click();
      await page.getByRole('switch', { name: 'Auto paste', exact: true }).uncheck();
    }
    await page.getByRole('searchbox').fill(query);
    await page.getByRole('button', { name: new RegExp(`^${query}`) }).last().click();
    const target = page.locator(`[data-setting-target="${parent}"]`);
    await expect(target).toHaveClass(/settings-target-flash/);
    await expect(target.getByRole('switch')).not.toBeChecked();
    if (child !== parent) await expect(page.locator(`[data-setting-target="${child}"]`).filter({ visible: true })).toHaveCount(0);
    await expect(target).toHaveCSS('animation-name', 'settings-search-target-pulse');
  });
}

test('enabling a feature reveals its explicit nav destination and closing a nav editor restores that caller', async ({ page }) => {
  await open(page, 'text');
  await expect(page.locator('.settings-sidebar').getByRole('button', { name: 'Project scan', exact: true })).toHaveCount(0);
  await advanced(page).click();
  await page.getByRole('switch', { name: 'Developer terms', exact: true }).check();
  const nav = page.locator('.settings-sidebar').getByRole('button', { name: 'Project scan', exact: true });
  await nav.click();
  await expect(page.getByRole('dialog', { name: 'Project scan' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(nav).toBeFocused();
  await expect(page.getByRole('heading', { name: 'Text & vocabulary', level: 1 })).toBeVisible();
});

for (const appearance of ['light', 'dark']) {
  test(`${appearance} app IDE rows and manual Advanced retain saved values`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce', colorScheme: appearance as 'light' | 'dark' });
    await page.goto(`/visual-fixtures.html?state=settings-folding&page=delivery&appearance=${appearance}&profile=1`);
    await advanced(page).click();
    await page.getByRole('button', { name: 'Manage app overrides', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'App overrides' });
    const parent = dialog.getByRole('switch', { name: 'Local IDE project context for Fixture editor' });
    await parent.scrollIntoViewIfNeeded();
    await expect(dialog.getByText('/fixture/project', { exact: true })).toBeHidden();
    await expect(page).toHaveScreenshot(`${appearance}-app-ide-folded.png`);
    await parent.click();
    await expect(dialog.getByText('/fixture/project', { exact: true })).toBeVisible();
    await expect(page).toHaveScreenshot(`${appearance}-app-ide-unfolded.png`);
    await parent.click();
    await parent.click();
    await expect(dialog.getByText('/fixture/project', { exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: /^Advanced/ }).click();
    const name = dialog.getByPlaceholder('App name (optional)');
    await name.fill('Draft app name');
    await dialog.getByRole('button', { name: /^Advanced/ }).click();
    await expect(name).toHaveCount(0);
    await dialog.getByRole('button', { name: /^Advanced/ }).click();
    await expect(name).toHaveValue('Draft app name');
  });
}

test('search opens the app editor Advanced and preserves permission/configuration boundaries', async ({ page }) => {
  await open(page, 'delivery', 'light', 'no-preference');
  await page.getByRole('searchbox').fill('App bundle ID');
  await page.getByRole('button', { name: /^App bundle ID/ }).click();
  const dialog = page.getByRole('dialog', { name: 'App overrides' });
  await expect(dialog.getByPlaceholder('com.apple.Terminal')).toBeVisible();
  await expect(dialog.getByRole('button', { name: /^Advanced/ })).toHaveAttribute('aria-expanded', 'true');
  await expect(dialog.locator('[data-setting-target=app-overrides-manual]')).toHaveClass(/settings-target-flash/);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { level: 1, name: 'Delivery' })).toBeVisible();
});

test('search reveals Advanced once and lets the user close it again', async ({ page }) => {
  await open(page, 'text');
  await page.getByRole('searchbox').fill('Project scan');
  await page.getByRole('button', { name: /^Project scan/ }).last().click();
  await expect(advanced(page)).toHaveAttribute('aria-expanded', 'true');
  await advanced(page).click();
  await expect(advanced(page)).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByRole('switch', { name: 'Developer terms', exact: true })).toHaveCount(0);
  await page.getByRole('searchbox').fill('Project scan');
  await page.getByRole('button', { name: /^Project scan/ }).last().click();
  await expect(advanced(page)).toHaveAttribute('aria-expanded', 'true');
});

test('enabled feature navigation has exactly one current page', async ({ page }) => {
  await page.goto('/visual-fixtures.html?state=settings-shortcuts&appearance=light');
  const nav = page.getByRole('navigation', { name: 'Settings pages' });
  for (const name of ['Voice Query', 'Selected-text rewrite']) {
    await nav.getByRole('button', { name, exact: true }).click();
    await expect(nav.locator('[aria-current="page"]')).toHaveCount(1);
    await expect(nav.getByRole('button', { name, exact: true })).toHaveAttribute('aria-current', 'page');
  }
});
