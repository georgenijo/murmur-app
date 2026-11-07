// Installed before application code, in every frame of isolated bundles only.
// Never retain or fall back to WebKit's general-pasteboard implementation.
(() => {
  const invoke = (command, args) => {
    const bridge = window.__TAURI_INTERNALS__;
    if (!bridge?.invoke) return Promise.reject(new Error('Private clipboard unavailable'));
    return bridge.invoke(`plugin:qa-clipboard|${command}`, args);
  };
  const readText = () => invoke('read_text');
  const writeText = text => invoke('write_text', { text: String(text) });
  const clipboard = Object.freeze({
    readText,
    writeText,
    read: async () => {
      const text = await readText();
      return [new ClipboardItem({ 'text/plain': new Blob([text], { type: 'text/plain' }) })];
    },
    write: async items => {
      if (items.length !== 1 || !items[0].types.includes('text/plain')) {
        throw new Error('Private clipboard supports one text item');
      }
      await writeText(await (await items[0].getType('text/plain')).text());
    },
  });
  Object.defineProperty(navigator, 'clipboard', {
    value: clipboard,
    configurable: false,
    writable: false,
  });
})();
