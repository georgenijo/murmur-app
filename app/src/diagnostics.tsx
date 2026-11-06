import { initializeWebkitState } from './lib/webkitLifecycle';
import React from 'react';
import ReactDOM from 'react-dom/client';
import { DiagnosticsWindowApp } from './components/log-viewer/DiagnosticsWindowApp';
import { hydrateSettingsFromDisk } from './lib/settings';
import { NativeWindowLifecycle } from './lib/windowLifecycle';
import './styles.css';

const root = ReactDOM.createRoot(document.getElementById('root') as HTMLElement);

// Seed localStorage from the durable settings.json before the first render.
initializeWebkitState().then(() => hydrateSettingsFromDisk()).then(() => {
  root.render(
    <React.StrictMode>
      <NativeWindowLifecycle showAfterPaint={false}>
        <DiagnosticsWindowApp />
      </NativeWindowLifecycle>
    </React.StrictMode>,
  );
});
