import { expect, test, type Page } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

async function settings(page: Page, editor: string, appearance: 'light' | 'dark') {
  await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=${editor}&appearance=${appearance}`);
  await expect(page.locator('.settings-editor-sheet > div:first-child h2')).toBeVisible();
}

for (const { appearance, width, height, label } of surfaceCases()) {
  test.describe(`Settings editors ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    test('Armed alias deletion blocks every modified Return activation', async ({ page }) => {
      await settings(page, 'aliases', appearance);
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
    test('Command editor traps Tab, preserves dirty Escape, and restores its opener', async ({ page }) => {
      await settings(page, 'commands', appearance);
      const create = page.getByRole('button', { name: 'New command…', exact: true });
      await create.click();
      const dialog = page.locator('[role="dialog"]:not(.settings-editor-sheet)');
      const first = dialog.getByRole('button', { name: 'Close voice command editor', exact: true });
      await expect(first).toBeFocused();
      await page.keyboard.press('Shift+Tab');
      await expect(dialog.getByRole('button', { name: 'Save command', exact: true })).toBeFocused();
      await page.keyboard.press('Tab');
      await expect(first).toBeFocused();
      // A pristine draft can still close directly and return to its real invoker.
      await page.keyboard.press('Escape');
      await expect(dialog).toHaveCount(0);
      await expect(create).toBeFocused();
      await create.click();
      await expect(first).toBeFocused();
      await dialog.getByLabel('Voice command type', { exact: true }).selectOption('snippet');
      await dialog.getByLabel('Voice command content', { exact: true }).fill('line one');
      await page.keyboard.press('Shift+Enter');
      const content = dialog.getByLabel('Voice command content', { exact: true });
      await expect(content).toHaveValue('line one\n');
      await page.keyboard.press('Escape');
      const unsaved = dialog.getByRole('group', { name: 'Unsaved voice command changes', exact: true });
      await expect(unsaved).toBeVisible();
      await expect(unsaved.getByRole('button', { name: 'Keep editing', exact: true })).toBeFocused();
      await expect(content).toHaveValue('line one\n');
      await expect(unsaved.getByRole('button', { name: 'Discard changes', exact: true })).toBeVisible();
      await unsaved.getByRole('button', { name: 'Keep editing', exact: true }).click();
      await expect(unsaved).toHaveCount(0);
      await expect(content).toBeFocused();
      await expect(content).toHaveValue('line one\n');
      await page.keyboard.press('Escape');
      await unsaved.getByRole('button', { name: 'Discard changes', exact: true }).click();
      await expect(dialog).toHaveCount(0);
      await expect(create).toBeFocused();
      await create.click();
      await expect(dialog.getByLabel('Voice command content', { exact: true })).toHaveValue('');
    });
    test('Knowledge deletion traps focus and restores its overflow trigger', async ({ page }) => {
      await settings(page, 'knowledge', appearance);
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
    test('Command editor keeps actions and final field visible; cancelled draft and owner handoff', async ({ page }) => {
      await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=commands&appearance=${appearance}`);
      await page.getByRole('button', { name: 'New command…', exact: true }).click();
      const dialog = page.getByRole('dialog', { name: 'Create voice command', exact: true });
      const save = dialog.getByRole('button', { name: 'Save command', exact: true });
      const cancel = dialog.getByRole('button', { name: 'Cancel', exact: true });
      for (const action of [save, cancel]) {
        await expect(action).toBeInViewport({ ratio: 1 });
        const box = (await action.boundingBox())!;
        expect(box.y + box.height).toBeLessThanOrEqual(height);
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
  });
}
