import { DictationSettingsNotice } from './components/DictationSettingsNotice';
import { SourceRecovery } from './components/SourceRecovery';
import { SourceStorageNotice } from './components/SourceStorageNotice';
import { initializeWebkitState } from './lib/webkitLifecycle';
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { CompletionNotice } from "./components/CompletionNotice";
import { flog } from "./lib/log";
import { NativeWindowLifecycle } from "./lib/windowLifecycle";
import { AppearanceProvider } from "./lib/hooks/useAppearance";
import { hydrateSettingsFromDisk, ownSettingsMigrations } from "./lib/settings";
import {
  hydrateThemeLibraryFromDisk,
  hydrateUserDataFromDisk,
} from "./lib/durableUserData";
import "./styles.css";

// Only the main entry owns the document viewport; companion windows share CSS.
document.documentElement.classList.add("murmur-main-window");
ownSettingsMigrations();

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

// Finish the main-window v1 importer and hydrate presentation snapshots before
// render. Settings/theme retain their existing ownership; completion data never
// enters a localStorage cache.
initializeWebkitState(true).then(() => Promise.all([
  hydrateSettingsFromDisk(),
  hydrateUserDataFromDisk(),
  hydrateThemeLibraryFromDisk(),
])).then(() => {
  flog.info('main', 'Frontend hydrated', { event_code: 'system.launch_frontend_hydrated' });
  root.render(
    <React.StrictMode>
      <AppearanceProvider>
        <NativeWindowLifecycle>
          <CompletionNotice />
          <SourceStorageNotice />
          <DictationSettingsNotice />
          <App />
        </NativeWindowLifecycle>
      </AppearanceProvider>
    </React.StrictMode>,
  );
}).catch(() => {
  // Preserve the source and retry on relaunch; never initialize empty defaults.
  root.render(<SourceRecovery />);
});
