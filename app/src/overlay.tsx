import React from "react";
import ReactDOM from "react-dom/client";
import { OverlayWidget } from "./components/OverlayWidget";
import { hydrateSettingsFromDisk } from "./lib/settings";
import "./styles.css";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

// Seed localStorage from the durable settings.json before the first render, so
// the overlay's synchronous settings mirror sees the durable copy.
hydrateSettingsFromDisk().finally(() => {
  root.render(
    <React.StrictMode>
      <OverlayWidget />
    </React.StrictMode>,
  );
});
