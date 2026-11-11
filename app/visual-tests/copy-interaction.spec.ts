import { expect, test } from '@playwright/test';

const panes = ['general', 'recording', 'delivery', 'meetings', 'text', 'ai', 'ai-transcription', 'appearance', 'performance', 'diagnostics', 'shortcuts'];
const editors = ['aliases', 'knowledge', 'commands', 'scan', 'vocabulary'];
const paneTitles: Record<string, string> = {
  general: 'General', recording: 'Recording', delivery: 'Delivery', meetings: 'Meetings',
  text: 'Text & vocabulary', ai: 'AI & models', 'ai-transcription': 'Speech-to-text',
  appearance: 'Appearance', performance: 'Performance Lab',
  diagnostics: 'Diagnostics', shortcuts: 'Keyboard shortcuts',
  'editor-aliases': 'Aliases', 'editor-knowledge': 'Knowledge',
  'editor-commands': 'Voice commands', 'editor-scan': 'Project scan', 'editor-vocabulary': 'Vocabulary',
};
for (const appearance of ['light', 'dark']) {
  for (const pane of [...panes, ...editors.map((editor) => `editor-${editor}`)]) {
    test(`${appearance} copy audit ${pane}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: appearance as 'light' | 'dark', reducedMotion: 'reduce' });
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      const editor = pane.startsWith('editor-') ? pane.slice(7) : null;
      await page.goto(`/visual-fixtures.html?state=settings-copy&appearance=${appearance}&page=${editor ? 'text' : pane}${editor ? `&editor=${editor}` : ''}`);
      await expect(page.locator('[data-visual-ready=true]')).toBeVisible();
      const heading = page.locator(editor ? '.settings-editor-sheet > div:first-child h2' : '.settings-workspace .settings-page-title');
      await expect(heading).toBeVisible();
      await expect(heading).toHaveText(paneTitles[pane]);
      // Real panes may retain several viewport lengths of advanced controls.
      const content = page.locator(editor ? '.settings-editor-sheet' : '[data-testid=settings-content]');
      const triples = await content.innerText();
      expect(triples).not.toContain('...');
      expect(triples).not.toContain('Text & Vocabulary');
      expect(triples).not.toContain('AI & Models');
      const advanced = content.locator('.settings-advanced-toggle');
      // Collapsed nested controls remain mounted but hidden until their parent
      // is revealed; expand only controls a user can actually reach.
      let advancedCount = 0;
      for (const button of await advanced.all()) {
        if (await button.isVisible() && await button.getAttribute('aria-expanded') === 'false') {
          await button.click();
          await expect(button).toHaveAttribute('aria-expanded', 'true');
          advancedCount++;
        }
      }
      const summaries = content.locator('summary');
      for (const summary of await summaries.all()) {
        if (await summary.isVisible() && !(await summary.locator('..').getAttribute('open') !== null)) {
          await summary.click();
          await expect(summary.locator('..')).toHaveAttribute('open', '');
        }
      }
      if (advancedCount || await summaries.count()) {
        await content.evaluate(async (element) => {
          // Let an Advanced branch mount and apply its revealed-row inset before
          // pinning the audit at the bottom of the scrollport.
          await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
          element.scrollTop = element.scrollHeight;
        });
      }
    });
  }

  test(`${appearance} recoverable default and destructive confirmation`, async ({ page }) => {
    await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=commands&appearance=${appearance}`);
    const create = page.getByRole('button', { name: 'New command…', exact: true });
    await create.focus(); await page.keyboard.press('Enter');
    await expect(page.locator('[role="dialog"]:not(.settings-editor-sheet)')).toHaveCount(0);
    await page.keyboard.press('Space');
    const dialog = page.locator('[role="dialog"]:not(.settings-editor-sheet)');
    await expect(dialog).toBeVisible();
    await dialog.getByLabel('Voice command phrase', { exact: true }).fill('insert farewell');
    await dialog.getByLabel('Voice command content', { exact: true }).fill('See you soon');
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).focus();
    await page.keyboard.press('Enter');
    await expect(dialog).toHaveCount(0);

    await page.goto(`/visual-fixtures.html?state=settings-copy&page=text&editor=knowledge&appearance=${appearance}`);
    await page.getByRole('button', { name: 'More knowledge actions' }).click();
    await page.getByRole('menuitem', { name: 'Delete all…', exact: true }).click();
    await page.getByLabel('Type DELETE to confirm', { exact: true }).fill('DELETE');
    const remove = page.getByRole('button', { name: 'Delete everything', exact: true });
    await expect(remove).toBeEnabled();
    await remove.focus();
    await page.keyboard.press('Enter');
    await expect(dialog).toBeVisible();
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
    await page.keyboard.press('Space');
    await expect(page.getByRole('button', { name: 'Delete Murmur…', exact: true })).toHaveCount(0);
  });
}
