import { describe, expect, it } from 'vitest';
import type { VocabularyEntry } from './settings';
import { normalizeVocabularyValue, validateVocabularyEntries } from './vocabulary';

function entry(written: string, aliases: string[]): VocabularyEntry {
  return {
    id: written,
    written,
    aliases,
    enabled: true,
    scope: { kind: 'global' },
  };
}

describe('validateVocabularyEntries', () => {
  it('matches backend ambiguity wording and rejects three-node cycles', () => {
    expect(validateVocabularyEntries([
      entry('Tauri', ['Tori']),
      entry('Tory CLI', ['Tori']),
    ])).toBe("Spoken alias 'Tori' is ambiguous between 'Tauri' and 'Tory CLI'.");

    expect(validateVocabularyEntries([
      entry('Alpha', ['Beta']),
      entry('Beta', ['Gamma']),
      entry('Gamma', ['Alpha']),
    ])).toBe('Cyclic aliases are not allowed. A spoken alias cannot lead back to its starting written term.');
  });

  it('allows the same alias in disjoint app scopes and rejects command collisions', () => {
    const terminal = entry('Tauri', ['Tori']);
    terminal.scope = { kind: 'app', bundleId: 'com.apple.Terminal' };
    const editor = entry('Story', ['Tori']);
    editor.scope = { kind: 'app', bundleId: 'com.example.Editor' };
    expect(validateVocabularyEntries([terminal, editor])).toBeNull();
    expect(validateVocabularyEntries([entry('LineBreak', ['new line'])])).toContain('Voice Command');
  });

  it('bounds alias values and rejects duplicate aliases deterministically', () => {
    expect(validateVocabularyEntries([entry('Tauri', ['Tori', 'tori'])]))
      .toBe("Spoken alias 'tori' is duplicated for 'Tauri'.");
    expect(validateVocabularyEntries([entry('Tauri', ['x'.repeat(257)])]))
      .toBe("The spoken alias for 'Tauri' is too long.");
  });

  it('normalizes Turkish and Azeri casing without the host locale', () => {
    expect(normalizeVocabularyValue(' I ')).toBe('i');
    expect(normalizeVocabularyValue('İ')).toBe('i\u0307');
    expect(validateVocabularyEntries([entry('Tauri', ['I', 'i'])]))
      .toBe("Spoken alias 'i' is duplicated for 'Tauri'.");
  });
});
