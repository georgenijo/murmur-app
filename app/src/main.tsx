import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { AppearanceProvider } from "./lib/hooks/useAppearance";
import { hydrateSettingsFromDisk } from "./lib/settings";
import { hydrateUserDataFromDisk } from "./lib/durableUserData";
import "./styles.css";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

// Seed synchronous localStorage caches from their Rust-owned durable copies
// before the first render. History and stats are main-window data; settings is
// hydrated by every window entry because overlay rendering also consumes it.
Promise.all([hydrateSettingsFromDisk(), hydrateUserDataFromDisk()]).finally(() => {
  root.render(
    <React.StrictMode>
      <AppearanceProvider>
        <App />
      </AppearanceProvider>
    </React.StrictMode>,
  );
});
