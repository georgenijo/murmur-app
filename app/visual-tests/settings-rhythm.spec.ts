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
];
for (const { path, state } of pages) {
  for (const expanded of [false, true]) {
    test(`${path.join(' / ')} ${state} ${expanded ? 'expanded' : 'default'} uses full-width card rows`, async ({ page }) => {
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(`/visual-fixtures.html?state=${state}&appearance=light`);
      await page.getByRole('navigation', { name: 'Settings pages' }).getByRole('button', { name: path[0], exact: true }).click();
      if (path[1]) await page.locator('main').getByRole('button', { name: new RegExp(`^${path[1]}`) }).first().click();
      await expect(page.locator('.settings-card-body').first()).toBeVisible();
      if (expanded) {
        for (let pass = 0; pass < 3; pass++) {
          const owners = page.locator('.settings-setting-row:has(+ .settings-dependent-branch) [role=switch][aria-checked=false]:not([data-disabled])');
          for (const owner of await owners.all()) if (await owner.isVisible()) await owner.click();
          for (const button of await page.locator('.settings-advanced-toggle[aria-expanded=false]').all()) await button.click();
        }
      }
      // A toggle can return before React and :has() layout settle on macOS.
      // Retry the same geometry contract until the revealed rows finish rendering.
      await expect(async () => {
        const result = await page.locator('.settings-card-body').evaluateAll(bodies => bodies.map(body => {
          const failures: string[] = [];
          const card = body.getBoundingClientRect();
          const cardStyle = getComputedStyle(body);
          if (cardStyle.padding !== '0px') failures.push(`card padding ${cardStyle.padding}`);
          [...body.children].forEach((row, i) => {
            const style = getComputedStyle(row);
            const bounds = row.getBoundingClientRect();
            const branch = row.classList.contains('settings-dependent-branch');
            const groupedAdvanced = row.matches('.settings-groups .settings-advanced');
            const advancedRows = row.matches('.settings-advanced:has(> .settings-advanced-rows)');
            const label = row.textContent?.trim().slice(0, 40);
            if (style.paddingTop !== (branch || groupedAdvanced ? '0px' : '7px') || style.paddingBottom !== (branch || groupedAdvanced || advancedRows ? '0px' : '7px')) failures.push(`${label}: vertical padding ${style.paddingTop}/${style.paddingBottom}`);
            // Advanced keeps its trailing 7px on the toggle in both states,
            // so row width and spacing do not change at animation boundaries.
            if (advancedRows && getComputedStyle(row.querySelector('.settings-advanced-toggle')!).marginBottom !== '7px') failures.push(`${label}: Advanced toggle spacing`);
            if (style.paddingLeft !== (branch ? '0px' : '16px') || style.paddingRight !== (branch ? '0px' : '16px')) failures.push(`${label}: horizontal padding ${style.paddingLeft}/${style.paddingRight}`);
            if (style.borderRadius !== '0px' && !row.matches('button:last-child')) failures.push(`${label}: rounded row ${style.borderRadius}`);
            if (Math.abs(bounds.width - (card.width - 2)) > .1) failures.push(`${label}: inset divider (${bounds.width}/${card.width})`);
            if (i > 0 && style.borderTopWidth !== (branch || groupedAdvanced ? '0px' : '1px')) failures.push(`${label}: divider ${style.borderTopWidth}`);
          });
          for (const disclosure of body.querySelectorAll('.settings-advanced:has(> .settings-advanced-toggle[aria-expanded="true"])')) {
            const toggle = disclosure.querySelector('.settings-advanced-toggle')!.getBoundingClientRect();
            const rows = disclosure.querySelector(':scope > .settings-advanced-rows > .settings-dependent-branch-clip > .settings-dependent-branch-content');
            if (!rows) continue; // Stack disclosures intentionally keep inset cards.
            for (const row of rows.children) {
              const bounds = row.getBoundingClientRect();
              const style = getComputedStyle(row);
              const label = row.textContent?.trim().slice(0, 40);
              if (Math.abs(bounds.left - (card.left + 1)) > .1 || Math.abs(bounds.right - (card.right - 1)) > .1) failures.push(`${label}: Advanced row edges`);
              if (row.classList.contains('settings-dependent-branch')) continue;
              if (Math.abs(bounds.left + parseFloat(style.paddingLeft) - toggle.left) > .1) failures.push(`${label}: Advanced content inset`);
              if (style.paddingTop !== '7px' || style.paddingBottom !== '7px' || style.borderTopWidth !== '1px') failures.push(`${label}: Advanced row rhythm`);
            }
            const last = rows.lastElementChild?.getBoundingClientRect();
            if (last && Math.abs(last.bottom - disclosure.getBoundingClientRect().bottom) > .1) failures.push('Advanced trailing inset');
          }
          return { failures, count: body.children.length };
        }));
        for (const card of result) {
          expect(card.count).toBeGreaterThan(0);
          expect(card.failures).toEqual([]);
        }
      }).toPass({ timeout: 5_000 });
    });
  }
}
