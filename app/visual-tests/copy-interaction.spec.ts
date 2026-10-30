import { expect, test } from '@playwright/test';

const panes = ['general', 'recording', 'delivery', 'meetings', 'text', 'ai', 'ai-transcription', 'ai-transform', 'ai-query', 'modes', 'appearance', 'performance', 'diagnostics', 'shortcuts'];
const editors = ['aliases', 'knowledge', 'commands', 'transforms', 'scan', 'vocabulary'];
for (const appearance of ['light', 'dark']) {
  for (const pane of [...panes, ...editors.map((editor) => `editor-${editor}`)]) {
    test(`${appearance} copy audit ${pane}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark' });
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      const editor = pane.startsWith('editor-') ? pane.slice(7) : null;
      await page.goto(`/visual-fixtures.html?state=settings-copy&appearance=${appearance}&page=${editor ? 'text' : pane}${editor ? `&editor=${editor}` : ''}`);
      await expect(page.locator('[data-visual-ready=true]')).toBeVisible();
      await expect(page.locator(editor ? '#settings-editor-title' : '.settings-workspace .settings-page-title')).toBeVisible();
      // Real panes may retain several viewport lengths of advanced controls.
      await expect(page.locator('[data-testid=settings-content]')).toHaveScreenshot(`${appearance}-copy-${pane}.png`);
      const triples = await page.locator('[data-testid=settings-content]').innerText();
      expect(triples).not.toContain('...');
      expect(triples).not.toContain('Text & Vocabulary');
      expect(triples).not.toContain('AI & Models');
      const summaries = page.locator('[data-testid=settings-content] summary');
      for (const summary of await summaries.all()) {
        if (await summary.isVisible() && !(await summary.locator('..').getAttribute('open') !== null)) await summary.click();
      }
      if (await summaries.count()) {
        await page.locator('[data-testid=settings-content]').evaluate((element) => { element.scrollTop = element.scrollHeight; });
        await expect(page.locator('[data-testid=settings-content]')).toHaveScreenshot(`${appearance}-copy-${pane}-expanded.png`);
      }
    });
  }

  test(`${appearance} recoverable default and destructive confirmation`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=commands&appearance=${appearance}`);
    const create = page.getByRole('button', { name: 'New command…', exact: true });
    await create.focus(); await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.keyboard.press('Space');
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await dialog.getByLabel('Voice command phrase', { exact: true }).fill('insert farewell');
    await dialog.getByLabel('Voice command content', { exact: true }).fill('See you soon');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).focus();
    await expect(dialog).toHaveScreenshot(`${appearance}-copy-command-default.png`);
    await page.keyboard.press('Enter');
    await expect(dialog).toHaveCount(0);

    await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=knowledge&appearance=${appearance}`);
    await page.getByRole('button', { name: 'More knowledge actions' }).click();
    await expect(page.getByRole('menu')).toHaveScreenshot(`${appearance}-copy-knowledge-overflow.png`);
    await page.getByRole('menuitem', { name: 'Delete all…', exact: true }).click();
    await page.getByLabel('Type DELETE to confirm', { exact: true }).fill('DELETE');
    const remove = page.getByRole('button', { name: 'Delete everything…', exact: true });
    await expect(remove).toBeEnabled();
    await remove.focus();
    await page.keyboard.press('Enter');
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveScreenshot(`${appearance}-copy-knowledge-delete.png`);
    await page.keyboard.press('Space');
    await expect(dialog).toHaveCount(0);
  });

  test(`${appearance} armed alias delete ignores Return and respects Space`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=aliases&appearance=${appearance}`);
    const remove = page.getByRole('button', { name: 'Delete Murmur…', exact: true });
    await remove.focus(); await page.keyboard.press('Space');
    const confirm = page.getByRole('button', { name: 'Confirm delete Murmur…', exact: true });
    await expect(confirm).toBeVisible();
    const actionColumn = await page.locator('[aria-label="Saved spellings"] > div').first().evaluate((element) => getComputedStyle(element).gridTemplateColumns.split(' ').at(-1));
    expect(actionColumn).toBe('120px');
    const toggleBounds = await page.getByRole('switch').first().boundingBox();
    const deleteBounds = await confirm.boundingBox();
    expect(toggleBounds!.x + toggleBounds!.width).toBeLessThanOrEqual(deleteBounds!.x);
    await page.keyboard.press('Enter'); await expect(confirm).toBeVisible();
    await expect(page.locator('[data-testid=settings-content]')).toHaveScreenshot(`${appearance}-copy-alias-armed.png`);
    await page.keyboard.press('Space');
    await expect(page.getByRole('button', { name: 'Delete Murmur…', exact: true })).toHaveCount(0);
  });
}
