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
    `emitTo({ kind: 'Any' }, 'e');`,
    `emitTo(target, 'e');`,
    `import { emitTo as send } from '@tauri-apps/api/event'; send({ kind: 'Any' }, 'e');`,
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

test('the sanctioned helper can forward a label, but cannot broadcast', () => {
  const helper = 'app/src/lib/targetedEvents.ts';
  assert.deepEqual(violations(`import { emitTo as send } from '@tauri-apps/api/event'; send(label, 'e');`, helper), []);
  assert.notDeepEqual(violations(`import { emitTo as send } from '@tauri-apps/api/event'; send({kind: 'Any'}, 'e');`, helper), []);
});

test('rejects direct event IPC, including constant folding and known dynamic prefixes', async (t) => {
  for (const source of [
    `invoke('plugin:event|emit', { event, payload });`,
    `invoke('plugin:event|emit_to', { target: { kind: 'Any' }, event, payload });`,
    'invoke(`plugin:event|emit`, { event, payload });',
    'invoke(`plugin:event|${"emit_to"}`, { event, payload });',
    `invoke('plugin:e' + 'vent|emit', { event, payload });`,
    String.raw`invoke('plugin:event|\x65mit_to', args);`,
    'invoke(`plugin:event|emit${suffix}`, args);',
    `const command = ('plugin:event|' + 'emit_') + suffix; ipc(command, args);`,
    `const command = 'plugin:event|emit_future_api'; transport(command, args);`,
    `invoke(('plugin:event|e' as const) + 'mit', args);`,
    'invoke(`plugin:event|${("em" as const) + "it_to"}`, args);',
  ]) await t.test(source, () => assert.notDeepEqual(violations(source), [], source));
});

test('transport values cannot escape target validation', async (t) => {
  for (const source of [
    `const send = emitTo; send({kind: 'Any'}, 'e');`,
    `import { emitTo as send } from '@tauri-apps/api/event'; const relay = send; relay(target, 'e');`,
    `import { emitTo as send } from '@tauri-apps/api/event.js'; send({kind: 'Any'}, 'e');`,
    `export { emitTo as send } from '@tauri-apps/api/event';`,
    `export { emitTo as send } from './local';`,
    `import { emitTo as send } from '@tauri-apps/api/event'; export { send };`,
    `consume(emitTo);`,
    `const transport = { send: emitTo };`,
    `const { emitTo: send } = window; send(target, 'e');`,
    `const send = window.emitTo; send(target, 'e');`,
    `const send = window['emitTo']; send(target, 'e');`,
    `Reflect.get(window, 'emitTo')(target, 'e');`,
    `emitTo.bind(null)({kind: 'Any'}, 'e');`,
    `emitTo.call(null, {kind: 'Any'}, 'e');`,
    `window['emitTo']({kind: 'Any'}, 'e');`,
    'window[`emit${"To"}`](target, "e");',
    `(0, emitTo)({kind: 'Any'}, 'e');`,
    `(emitTo as Function)(target, 'e');`,
    `import { sendTo as send } from './targetedEvents'; const relay = send; relay(target, 'e');`,
    `import { sendTo as send } from './targetedEvents'; export { send };`,
    `import { sendTo as send } from './targetedEvents'; send(target, 'e');`,
    `export * from './targetedEvents';`,
    `import * as helpers from './targetedEvents'; consume(helpers);`,
    `const helpers = await import('./targetedEvents'); consume(helpers);`,
  ]) await t.test(source, () => assert.notDeepEqual(violations(source), [], source));
});

test('direct imports and validated targets remain available', () => {
  assert.deepEqual(violations(`
    import { emitTo as send } from '@tauri-apps/api/event.js';
    send('main', 'e');
    window['emitTo']('overlay', 'e');
    import { sendTo as notify } from './targetedEvents';
    notify('diagnostics', 'e');
    invoke('plugin:event|listen', args);
    // invoke('plugin:event|emit', args);
  `), []);
  assert.deepEqual(violations(`invoke('plugin:event|emit_to', args);`, 'app/src/lib/targetedEvents.ts'), []);
  assert.notDeepEqual(violations(`import { emitTo as send } from '@tauri-apps/api/event'; consume(send);`, 'example.test.ts'), []);
});
