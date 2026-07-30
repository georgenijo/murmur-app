import React from "react";
import ReactDOM from "react-dom/client";
import { LogViewerApp } from "./components/log-viewer/LogViewerApp";
import { useAppearanceReader } from "./lib/hooks/useAppearance";
import "./styles.css";

function ThemedLogViewer() {
  useAppearanceReader();
  return <LogViewerApp />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ThemedLogViewer />
  </React.StrictMode>,
);
