import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

describe('appearance barrel', () => {
  it('does not statically re-export the lazy Open VSX importer', () => {
    // Every window imports this barrel. Re-exporting openVsxThemes would pull
    // jszip and @noble/hashes into the eager chunk and defeat the dynamic
    // import() in CommunityThemeDialog.
    const source = readFileSync(resolve(__dirname, 'index.ts'), 'utf8');
    expect(source).not.toMatch(/openVsxThemes/);
  });
});
