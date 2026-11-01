import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
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

// Seed synchronous localStorage caches from their Rust-owned durable copies
// before the first render. History and stats are main-window data; settings is
// hydrated by every window entry because overlay rendering also consumes it.
Promise.all([
  hydrateSettingsFromDisk(),
  hydrateUserDataFromDisk(),
  hydrateThemeLibraryFromDisk(),
]).finally(() => {
  root.render(
    <React.StrictMode>
      <AppearanceProvider>
        <NativeWindowLifecycle>
          <App />
        </NativeWindowLifecycle>
      </AppearanceProvider>
    </React.StrictMode>,
  );
});
