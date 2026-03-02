import React from "react";
import ReactDOM from "react-dom/client";
import { LogViewerApp } from "./components/log-viewer/LogViewerApp";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <LogViewerApp />
  </React.StrictMode>,
);
