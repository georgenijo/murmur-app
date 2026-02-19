import React from "react";
import ReactDOM from "react-dom/client";
import { OverlayWidget } from "./components/OverlayWidget";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <OverlayWidget />
  </React.StrictMode>,
);
