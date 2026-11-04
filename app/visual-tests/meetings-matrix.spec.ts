import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';

for (const { appearance, width, height, label } of surfaceCases([{ width: 720, height: 560 }, { width: 880, height: 820 }, { width: 1120, height: 820 }])) {
  test.describe(`Meetings ${label}`, () => {
    test.use({ viewport: { width, height }, colorScheme: appearance });
    test('Action edit fields fit; Cancel restores values and Save preserves canonical transcript', async ({ page }, testInfo) => {
      await page.goto(`/visual-fixtures.html?state=meetings-review&appearance=${appearance}`);
      const evidence = page.locator('.meeting-transcript-row');
      await expect(evidence.first()).toBeVisible();
      const canonical = await evidence.allTextContents();
      await page.getByRole('button', { name: 'Edit review…', exact: true }).click();
      const task = page.getByRole('textbox', { name: 'Action item 1', exact: true });
      const owner = page.getByRole('textbox', { name: 'Action owner 1', exact: true });
      const date = page.getByLabel('Action due date 1', { exact: true });
      await task.scrollIntoViewIfNeeded();
      const layout = await task.evaluate(element => {
        const editor = element.closest('.meeting-action-editor')!;
        const taskRect = element.getBoundingClientRect();
        const editorRect = editor.getBoundingClientRect();
        return { taskWidth: taskRect.width, editorWidth: editorRect.width, overflow: editor.scrollWidth - editor.clientWidth };
      });
      expect(layout.taskWidth).toBeGreaterThan(160);
      expect(layout.taskWidth).toBeGreaterThan(layout.editorWidth * 0.85);
      expect(layout.overflow).toBeLessThanOrEqual(1);
      for (const field of [owner, date, page.getByRole('button', { name: 'Remove action item 1', exact: true })]) {
        await field.scrollIntoViewIfNeeded();
        await expect(field).toBeInViewport();
        const box = await field.boundingBox();
        expect(box!.width).toBeGreaterThan(35);
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(width);
      }
      await task.fill('Draft task that will be cancelled');
      await owner.fill('Casey');
      await date.fill('2026-11-06');
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
      await page.getByRole('button', { name: 'Edit review…', exact: true }).click();
      await expect(task).toHaveValue('Verify every export format.');
      await expect(owner).toHaveValue('Alex');
      await expect(date).toHaveValue('');
      await task.fill('Verify the reviewed plan and all milestones');
      await owner.fill('Casey');
      await date.fill('2026-11-07');
      await task.scrollIntoViewIfNeeded();
      if (width !== 1120) await page.screenshot({ path: testInfo.outputPath('meeting-action-editor.png') });
      await page.getByRole('button', { name: 'Save review', exact: true }).click();
      await expect(page.getByRole('form', { name: 'Edit meeting review' })).toHaveCount(0);
      expect(await evidence.allTextContents()).toEqual(canonical);
    });
  });
}
