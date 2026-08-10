import React from "react";
import ReactDOM from "react-dom/client";
import { TransformReviewApp } from "./components/transform-review/TransformReviewApp";
import { hydrateSettingsFromDisk } from "./lib/settings";
import "./styles.css";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

// Seed localStorage from the durable settings.json before the first render.
hydrateSettingsFromDisk().finally(() => {
  root.render(
    <React.StrictMode>
      <TransformReviewApp />
    </React.StrictMode>,
  );
});
