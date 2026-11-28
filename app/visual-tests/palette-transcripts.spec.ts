import { expect, test } from '@playwright/test';

// Opt-in "Find transcripts in the command palette". Geometry only: transcript
// rows sit under their own heading inside the dialog, a long snippet truncates
// on one line instead of pushing the date label out, and the footer verb
// follows the selected row.
for (const appearance of ['light', 'dark'] as const) {
  for (const size of [{ width: 1100, height: 780 }, { width: 840, height: 620 }]) {
    test(`palette transcript rows stay inside the dialog (${appearance} ${size.width}x${size.height})`, async ({ page }) => {
      await page.setViewportSize(size);
      await page.clock.setFixedTime(new Date('2026-09-04T20:00:00Z'));
      await page.goto(`/visual-fixtures.html?state=palette-transcripts&appearance=${appearance}`);

      const dialog = page.getByRole('dialog', { name: 'Command palette' });
      await expect(dialog).toBeVisible();
      await expect(dialog.getByRole('combobox')).toHaveValue('transcript');
      await expect(dialog.getByRole('combobox')).toHaveAttribute('placeholder', 'Type a command or search transcripts…');
      await expect(dialog.locator('.dialog-eyebrow', { hasText: 'Transcripts' })).toHaveCount(1);

      const rows = dialog.locator('[id^="command-transcript-"]');
      await expect(rows).toHaveCount(5);
      const dialogBox = await dialog.boundingBox();
      expect(dialogBox).toBeTruthy();
      for (const row of await rows.all()) {
        const box = await row.boundingBox();
        const title = await row.locator('span.truncate').boundingBox();
        const meta = await row.locator('.palette-meta').boundingBox();
        expect(box && title && meta).toBeTruthy();
        expect(box!.x).toBeGreaterThanOrEqual(dialogBox!.x - 0.5);
        expect(box!.x + box!.width).toBeLessThanOrEqual(dialogBox!.x + dialogBox!.width + 0.5);
        // Title and date label share one line without overlapping.
        expect(title!.x + title!.width).toBeLessThanOrEqual(meta!.x + 0.5);
        expect(meta!.x + meta!.width).toBeLessThanOrEqual(box!.x + box!.width + 0.5);
        expect(box!.height).toBeLessThanOrEqual(44);
        await expect(row.locator('.palette-meta')).toContainText('·');
      }
      expect(await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);

      const footer = dialog.locator('.palette-footer');
      await expect(footer).toContainText('run');
      const input = dialog.getByRole('combobox');
      const firstTranscript = await rows.first().getAttribute('id');
      // Step down past any matching commands to the first transcript row.
      for (let step = 0; step < 10; step++) {
        if ((await input.getAttribute('aria-activedescendant')) === firstTranscript) break;
        await input.press('ArrowDown');
      }
      await expect(input).toHaveAttribute('aria-activedescendant', firstTranscript!);
      await expect(footer).toContainText('copy');
    });
  }
}
