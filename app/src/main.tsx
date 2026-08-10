import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { AppearanceProvider } from "./lib/hooks/useAppearance";
import { hydrateSettingsFromDisk } from "./lib/settings";
import "./styles.css";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

// Seed localStorage from the durable settings.json before the first render, so
// every synchronous `loadSettings()` below sees the durable copy.
hydrateSettingsFromDisk().finally(() => {
  root.render(
    <React.StrictMode>
      <AppearanceProvider>
        <App />
      </AppearanceProvider>
    </React.StrictMode>,
  );
});
