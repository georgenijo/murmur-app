import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { violations } from './check_targeted_frontend.mjs';

test('rejects broadcast imports, aliases, re-exports and window methods', () => {
  for (const source of [
    `import { emit } from '@tauri-apps/api/event'; emit('e');`,
    `import { emit as broadcast } from '@tauri-apps/api/event'; broadcast('e');`,
    `import * as events from '@tauri-apps/api/event'; events.emit('e');`,
    `export { emit as send } from '@tauri-apps/api/event';`,
    `export * from '@tauri-apps/api/event';`,
    `import * as windows from '@tauri-apps/api/window';`,
    `const transport = await import('@tauri-apps/api/event');`,
    `const transport = require('@tauri-apps/api/event');`,
    `getCurrentWebviewWindow().emit('e');`,
    `window.emit?.('e');`,
    `window['emit']('e');`,
    `window['em' + 'it']('e');`,
    'window["\\x65mit"]("e");',
    'window.\\u0065mit("e");',
    'const text = `before ${window.emit("e")} after`;',
    `const { emit: broadcast } = window;`,
    `const broadcast = window.emit;`,
    `Window.prototype.emit.call(window, 'e');`,
    `Reflect.get(window, 'emit')('e');`,
    `const { ['em' + 'it']: broadcast } = window;`,
    'window[`em${"it"}`]("e");',
  ]) assert.notDeepEqual(violations(source), [], source);
});
test('test helpers do not exempt actual broadcast imports', () => {
  assert.notDeepEqual(violations(`import { emit as broadcast } from '@tauri-apps/api/event';`, 'example.test.ts'), []);
  assert.deepEqual(violations(`const emit = () => {}; emit(); mocks.emit();`, 'example.test.ts'), []);
});
test('allows targeted transports, comments, literals and ordinary property access', () => {
  assert.deepEqual(violations(`
    import { emitTo } from '@tauri-apps/api/event';
    import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
    // getCurrentWebviewWindow().emit('e');
    const example = "window.emit('e')";
    const template = \`window.emit('e')\`;
    emitTo('overlay', 'e', { active: true });
  `), []);
});
