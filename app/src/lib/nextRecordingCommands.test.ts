import { describe, expect, it, vi } from 'vitest';
import { nextRecordingCommands } from './nextRecordingCommands';
import { filterCommands } from './commandPalette';

describe('next recording Mode palette commands', () => {
  it('uses backend enabled choices and sends the exact Mode ID without changing the binding', () => {
    const select = vi.fn();
    const commands = nextRecordingCommands({ id: 'builtin.email', name: 'Email', source: 'app_binding',
      available: [{ id: 'builtin.verbatim', name: 'Verbatim' }, { id: 'mode.custom', name: 'My Mode' }],
      pending: null,
    }, select);
    expect(commands.map((command) => command.title)).toEqual(['Next recording: Verbatim', 'Next recording: My Mode']);
    const matches = filterCommands(commands, 'next verbatim');
    expect(matches).toHaveLength(1);
    matches[0].run();
    expect(select).toHaveBeenCalledExactlyOnceWith('builtin.verbatim');
  });

  it('marks the pending selection and offers explicit cancellation only while pending', () => {
    const select = vi.fn();
    const commands = nextRecordingCommands({ id: 'builtin.email', name: 'Email', source: 'app_binding',
      available: [{ id: 'builtin.verbatim', name: 'Verbatim' }],
      pending: { id: 'builtin.verbatim', name: 'Verbatim' },
    }, select);
    expect(commands[0].hint).toBe('pending');
    commands[1].run();
    expect(select).toHaveBeenCalledExactlyOnceWith(null);
  });
});
