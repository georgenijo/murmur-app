import { expect, test, type Locator } from '@playwright/test';

const box = async (locator: Locator) => (await locator.boundingBox())!;

for (const appearance of ['light', 'dark'] as const) {
  test(`speech-to-text model card is plain, aligned and untruncated (${appearance})`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce', colorScheme: appearance });
    await page.goto(`/visual-fixtures.html?state=settings-folding&page=ai-transcription&runtime=failed&appearance=${appearance}`);
    await expect(page.locator('[data-visual-ready=true]')).toBeVisible();
    const rows = page.getByTestId('model-storage-row');
    await expect(rows).toHaveCount(3);

    const text = (await page.getByTestId('settings-content').innerText());
    for (const raw of ['notInstalled', 'coreml', 'unloaded', 'failed', 'Apple Neural Engine']) {
      expect(text).not.toContain(raw);
    }

    // The repair notice is one sentence with a primary action and a warning tone.
    const notice = page.getByTestId('model-notice');
    await expect(notice).toHaveAttribute('data-tone', 'warning');
    await expect(page.getByTestId('model-repair-message'))
      .toHaveText('Parakeet Core ML is incomplete, so dictation can’t run. Re-download to fix it.');
    const redownload = page.getByRole('button', { name: 'Re-download', exact: true });
    await expect(redownload).toHaveAttribute('data-variant', 'primary');

    // It spans the card, sits below the model select, and never overlaps it.
    const card = page.locator('.settings-card').first();
    const cardBox = await box(card);
    const noticeBox = await box(notice);
    const trigger = page.getByRole('combobox', { name: 'Transcription model' });
    const triggerBox = await box(trigger);
    expect(noticeBox.x).toBeGreaterThan(cardBox.x);
    expect(noticeBox.width).toBeGreaterThan(cardBox.width - 48);
    expect(noticeBox.x + noticeBox.width).toBeLessThanOrEqual(cardBox.x + cardBox.width);
    expect(noticeBox.y).toBeGreaterThanOrEqual(triggerBox.y + triggerBox.height);
    // The button sits on the notice's right edge and the text stays on one or two lines.
    const buttonBox = await box(redownload);
    expect(buttonBox.x + buttonBox.width).toBeLessThanOrEqual(noticeBox.x + noticeBox.width);
    expect(buttonBox.x).toBeGreaterThan(noticeBox.x + noticeBox.width / 2);
    expect(buttonBox.height).toBeLessThan(40);
    const wordLines = await page.getByTestId('model-repair-message').evaluate(
      (el) => el.querySelector('span')!.getClientRects().length,
    );
    expect(wordLines).toBe(1);

    // The selected model name is never ellipsised next to its badge.
    const label = trigger.locator('span.truncate');
    await expect(label).toHaveText('Parakeet Core ML');
    expect(await label.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(false);

    // Storage is one grouped list: selected first, no card per row, one action column.
    await expect(rows.nth(0)).toContainText('Parakeet Core ML');
    await expect(rows.nth(0)).toContainText('Incomplete');
    await expect(rows.nth(0)).toContainText('Selected');
    await expect(rows.nth(1).getByRole('button', { name: /^Remove/ })).toBeVisible();
    await expect(rows.nth(2).getByRole('button', { name: /^Download/ })).toBeVisible();
    for (let index = 0; index < 3; index += 1) {
      const row = rows.nth(index);
      expect(await row.evaluate((el) => parseFloat(getComputedStyle(el).borderTopLeftRadius))).toBe(0);
    }
    const actionRights = await rows.evaluateAll((elements) => elements.map((element) => {
      const action = element.querySelector('.model-storage-action')!.getBoundingClientRect();
      return Math.round(action.right);
    }));
    expect(new Set(actionRights).size).toBe(1);
    const controlRights = await rows.evaluateAll((elements) => elements.map((element) => {
      const control = element.querySelector('.model-storage-action > *')!.getBoundingClientRect();
      return Math.round(control.right);
    }));
    expect(new Set(controlRights).size).toBe(1);
  });
}
