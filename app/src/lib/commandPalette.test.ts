import { describe, it, expect, vi } from 'vitest';
import {
  filterCommands,
  isSubsequence,
  moveSelection,
  paletteTokens,
  scoreCommand,
  type PaletteCommand,
} from './commandPalette';

function command(id: string, title: string, section = 'General', keywords?: string[]): PaletteCommand {
  return { id, title, section, keywords, run: vi.fn() };
}

const COMMANDS: PaletteCommand[] = [
  command('record', 'Start recording', 'Recording', ['dictate', 'mic']),
  command('stop', 'Stop recording', 'Recording'),
  command('settings-delivery', 'Settings: Delivery', 'Settings', ['paste', 'clipboard']),
  command('settings-recording', 'Settings: Recording', 'Settings'),
  command('logs', 'Open performance diagnostics', 'Diagnostics', ['events', 'debug', 'log']),
  command('search', 'Search transcripts', 'History'),
];

describe('paletteTokens', () => {
  it('lowercases and splits, dropping empties', () => {
    expect(paletteTokens('  Open  LOG ')).toEqual(['open', 'log']);
    expect(paletteTokens('   ')).toEqual([]);
  });
});

describe('isSubsequence', () => {
  it('accepts in-order character runs', () => {
    expect(isSubsequence('slg', 'start log')).toBe(true);
    expect(isSubsequence('', 'anything')).toBe(true);
  });
  it('rejects out-of-order characters', () => {
    expect(isSubsequence('gls', 'start log')).toBe(false);
  });
});

describe('scoreCommand', () => {
  it('scores every command equally for an empty query', () => {
    expect(scoreCommand(COMMANDS[0], '')).toBe(0);
    expect(scoreCommand(COMMANDS[3], '   ')).toBe(0);
  });

  it('ranks prefix above substring above subsequence', () => {
    const prefix = scoreCommand(command('a', 'Delivery settings'), 'del')!;
    const substring = scoreCommand(command('b', 'Zzz delivery'), 'del')!;
    const subsequence = scoreCommand(command('c', 'Dump event log'), 'del')!;
    expect(prefix).toBeGreaterThan(substring);
    expect(substring).toBeGreaterThan(subsequence);
  });

  it('matches a word start inside the title', () => {
    expect(scoreCommand(command('a', 'Settings: Delivery'), 'delivery')).toBeGreaterThan(0);
  });

  it('matches keywords and sections below the title', () => {
    const byTitle = scoreCommand(COMMANDS[0], 'record')!;
    const byKeyword = scoreCommand(COMMANDS[0], 'dictate')!;
    expect(byTitle).toBeGreaterThan(byKeyword);
    expect(scoreCommand(COMMANDS[4], 'diagnostics')).toBeGreaterThan(0);
  });

  it('returns null when any token fails to match', () => {
    expect(scoreCommand(COMMANDS[0], 'start nonsense')).toBeNull();
    expect(scoreCommand(COMMANDS[0], 'zqxj')).toBeNull();
  });

  it('requires all tokens (AND, not OR)', () => {
    expect(scoreCommand(COMMANDS[2], 'settings delivery')).not.toBeNull();
    expect(scoreCommand(COMMANDS[3], 'settings delivery')).toBeNull();
  });

  it('prefers shorter titles on a tie', () => {
    const short = scoreCommand(command('a', 'Settings'), 'settings')!;
    const long = scoreCommand(command('b', 'Settings'), 'settings')!;
    expect(short).toBe(long);
    const shorter = scoreCommand(command('a', 'Log'), 'log')!;
    const longer = scoreCommand(command('b', 'Log viewer window'), 'log')!;
    expect(shorter).toBeGreaterThan(longer);
  });
});

describe('filterCommands', () => {
  it('returns everything in declaration order for an empty query', () => {
    expect(filterCommands(COMMANDS, '').map((c) => c.id)).toEqual(COMMANDS.map((c) => c.id));
  });

  it('puts the best match first', () => {
    expect(filterCommands(COMMANDS, 'stop')[0].id).toBe('stop');
    expect(filterCommands(COMMANDS, 'log')[0].id).toBe('logs');
    expect(filterCommands(COMMANDS, 'settings del')[0].id).toBe('settings-delivery');
  });

  it('finds a command through its keywords', () => {
    expect(filterCommands(COMMANDS, 'clipboard').map((c) => c.id)).toEqual(['settings-delivery']);
  });

  it('drops non-matching commands', () => {
    expect(filterCommands(COMMANDS, 'zzzz')).toEqual([]);
  });

  it('is stable for equal scores', () => {
    const twins = [command('first', 'Same title'), command('second', 'Same title')];
    expect(filterCommands(twins, 'same').map((c) => c.id)).toEqual(['first', 'second']);
  });
});

describe('moveSelection', () => {
  it('wraps in both directions', () => {
    expect(moveSelection(0, -1, 3)).toBe(2);
    expect(moveSelection(2, 1, 3)).toBe(0);
    expect(moveSelection(1, 1, 3)).toBe(2);
  });
  it('is safe on an empty list', () => {
    expect(moveSelection(0, 1, 0)).toBe(0);
  });
});
