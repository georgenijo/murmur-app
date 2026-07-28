(function () {
  'use strict';

  var STORAGE_KEY = 'murmur-appearance';
  var MAX_BYTES = 64 * 1024;
  var CACHE_VERSION = 1;
  var TOKEN_NAMES = [
    'background',
    'surface',
    'surface-container-low',
    'surface-container',
    'surface-container-high',
    'surface-container-lowest',
    'surface-container-highest',
    'primary',
    'primary-dim',
    'on-primary',
    'on-surface',
    'on-surface-variant',
    'outline-variant',
    'error',
    'success',
    'warning',
  ];
  var SONIC_LIGHT = {
    'background': '#f7fafc',
    'surface': '#f7fafc',
    'surface-container-low': '#eff4f8',
    'surface-container': '#e9eff3',
    'surface-container-high': '#e2e9ee',
    'surface-container-lowest': '#ffffff',
    'surface-container-highest': '#dbe4e9',
    'primary': '#036785',
    'primary-dim': '#005a75',
    'on-primary': '#f3faff',
    'on-surface': '#2b3438',
    'on-surface-variant': '#586065',
    'outline-variant': '#abb3b9',
    'error': '#a83836',
    'success': '#146333',
    'warning': '#654500',
  };
  var SONIC_DARK = {
    'background': '#0b0f11',
    'surface': '#0b0f11',
    'surface-container-low': '#151a1e',
    'surface-container': '#1e2529',
    'surface-container-high': '#283035',
    'surface-container-lowest': '#0f1315',
    'surface-container-highest': '#323b41',
    'primary': '#92dbfe',
    'primary-dim': '#84cdef',
    'on-primary': '#00394b',
    'on-surface': '#dbe4e9',
    'on-surface-variant': '#abb3b9',
    'outline-variant': '#586065',
    'error': '#fa746f',
    'success': '#66d99a',
    'warning': '#f4bd65',
  };

  function concreteMode(mode) {
    if (mode === 'light' || mode === 'dark') return mode;
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches
      ? 'dark'
      : 'light';
  }

  function validTable(table) {
    if (!table || typeof table !== 'object' || Array.isArray(table)) return false;
    var keys = Object.keys(table);
    if (keys.length !== TOKEN_NAMES.length) return false;
    for (var index = 0; index < TOKEN_NAMES.length; index += 1) {
      var value = table[TOKEN_NAMES[index]];
      if (typeof value !== 'string' || !/^#[0-9a-fA-F]{6}$/.test(value)) return false;
    }
    return true;
  }

  function apply(mode, tokens) {
    var root = document.documentElement;
    root.setAttribute('data-appearance', mode);
    root.style.colorScheme = mode;
    for (var index = 0; index < TOKEN_NAMES.length; index += 1) {
      var token = TOKEN_NAMES[index];
      root.style.setProperty('--murmur-' + token, tokens[token].toLowerCase());
    }
  }

  var stored = null;
  try {
    var raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw !== null) {
      var size = window.TextEncoder ? new window.TextEncoder().encode(raw).byteLength : raw.length;
      if (size <= MAX_BYTES) stored = JSON.parse(raw);
    }
  } catch (_) {
    stored = null;
  }

  var mode = concreteMode(stored && stored.version === 1 ? stored.mode : 'system');
  var fallback = mode === 'dark' ? SONIC_DARK : SONIC_LIGHT;
  var validTheme = stored
    && stored.version === 1
    && stored.theme
    && stored.theme.version === 1
    && (stored.theme.presetId === 'sonic' || stored.theme.presetId === 'custom');
  var cache = validTheme && stored.cache
    && stored.cache.version === CACHE_VERSION
    ? stored.cache[mode]
    : null;
  apply(mode, validTable(cache) ? cache : fallback);
}());
