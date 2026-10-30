import { expect, test } from '@playwright/test';

/** Measure production card edges and row dividers, including revealed branches.
 * The #848 contract replaces the former 6px outer card inset. */
test.use({ viewport: { width: 1180, height: 3200 } });
const pages = [
  { path: ['General'], state: 'settings' },
  { path: ['Recording'], state: 'settings' },
  { path: ['Recording'], state: 'settings-smart-auto' },
  { path: ['Delivery'], state: 'settings' },
  { path: ['Meetings'], state: 'settings' },
  { path: ['Text & vocabulary'], state: 'settings' },
  { path: ['AI & models'], state: 'settings' },
  { path: ['AI & models', 'Speech-to-text'], state: 'settings' },
  { path: ['AI & models', 'Voice Query'], state: 'settings' },
  { path: ['AI & models', 'Selected-text rewrite'], state: 'settings' },
];
for (const { path, state } of pages) {
  for (const expanded of [false, true]) {
    test(`${path.join(' / ')} ${state} ${expanded ? 'expanded' : 'default'} uses full-width card rows`, async ({ page }) => {
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(`/visual-fixtures.html?state=${state}&appearance=light`);
      await page.getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: path[0], exact: true }).click();
      if (path[1]) await page.locator('main').getByRole('button', { name: new RegExp(`^${path[1]}`) }).first().click();
      await expect(page.locator('.settings-card-body')).toBeVisible();
      if (expanded) {
        for (let pass = 0; pass < 3; pass++) {
          const owners = page.locator('.settings-setting-row:has(+ .settings-dependent-branch) [role=switch][aria-checked=false]:not([data-disabled])');
          for (const owner of await owners.all()) if (await owner.isVisible()) await owner.click();
          await page.locator('.settings-card-body details').evaluateAll(elements => elements.forEach(el => { (el as HTMLDetailsElement).open = true; }));
        }
      }
      const result = await page.locator('.settings-card-body').evaluate(body => {
        const failures: string[] = [];
        const card = body.getBoundingClientRect();
        const cardStyle = getComputedStyle(body);
        if (cardStyle.padding !== '0px') failures.push(`card padding ${cardStyle.padding}`);
        [...body.children].forEach((row, i) => {
          const style = getComputedStyle(row);
          const bounds = row.getBoundingClientRect();
          const branch = row.classList.contains('settings-dependent-branch');
          const label = row.textContent?.trim().slice(0, 40);
          if (style.paddingTop !== (branch ? '0px' : '12px') || style.paddingBottom !== (branch ? '0px' : '12px')) failures.push(`${label}: vertical padding ${style.paddingTop}/${style.paddingBottom}`);
          if (style.paddingLeft !== '16px' || style.paddingRight !== '16px') failures.push(`${label}: horizontal padding ${style.paddingLeft}/${style.paddingRight}`);
          if (style.borderRadius !== '0px') failures.push(`${label}: rounded row ${style.borderRadius}`);
          if (Math.abs(bounds.width - (card.width - 2)) > .1) failures.push(`${label}: inset divider (${bounds.width}/${card.width})`);
          if (i > 0 && style.borderTopWidth !== (branch ? '0px' : '1px')) failures.push(`${label}: divider ${style.borderTopWidth}`);
        });
        return { failures, count: body.children.length };
      });
      expect(result.count).toBeGreaterThan(0);
      expect(result.failures).toEqual([]);
    });
  }
}
