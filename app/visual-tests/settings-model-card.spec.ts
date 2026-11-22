import { expect, test } from '@playwright/test';

for (const appearance of ['light', 'dark'] as const) {
  test(`speech-to-text model card is plain and untruncated (${appearance})`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce', colorScheme: appearance });
    await page.goto(`/visual-fixtures.html?state=settings-folding&page=ai-transcription&runtime=failed&appearance=${appearance}`);
    await expect(page.locator('[data-visual-ready=true]')).toBeVisible();
    const rows = page.getByTestId('model-storage-row');
    await expect(rows).toHaveCount(3);

    const text = (await page.getByTestId('settings-content').innerText());
    for (const raw of ['notInstalled', 'coreml', 'unloaded', 'failed', 'Apple Neural Engine']) {
      expect(text).not.toContain(raw);
    }

    // A partial or failed install explains itself and offers a working repair action.
    await expect(page.getByText('Model files are incomplete. Re-download to repair.')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Re-download', exact: true })).toBeVisible();

    // The repair copy must not break inside "Re-download", and the button stays on one line.
    const message = page.getByTestId('model-repair-message');
    const words = await message.evaluate((el) => {
      const word = el.querySelector('span')!;
      return { wordLines: word.getClientRects().length, wordHeight: word.getBoundingClientRect().height };
    });
    expect(words.wordLines).toBe(1);
    const redownload = page.getByRole('button', { name: 'Re-download', exact: true });
    const lineHeight = await redownload.evaluate((el) => parseFloat(getComputedStyle(el).lineHeight) || 0);
    expect((await redownload.boundingBox())!.height).toBeLessThan(lineHeight * 2 + 8);

    // The selected model name is never ellipsised next to its badge.
    const trigger = page.getByRole('combobox', { name: 'Transcription model' });
    const label = trigger.locator('span.truncate');
    await expect(label).toHaveText('Parakeet Core ML');
    const clipped = await label.evaluate((el) => el.scrollWidth > el.clientWidth);
    expect(clipped).toBe(false);

    // Selected row carries a status chip instead of an empty action slot; others keep a button.
    await expect(rows.nth(0)).toContainText('Selected');
    await expect(rows.nth(1).getByRole('button', { name: /^Download/ })).toBeVisible();
    await expect(rows.nth(2).getByRole('button', { name: /^Remove/ })).toBeVisible();
  });
}
