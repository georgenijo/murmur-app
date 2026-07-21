import React from "react";
import ReactDOM from "react-dom/client";
import { TransformReviewApp } from "./components/transform-review/TransformReviewApp";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <TransformReviewApp />
  </React.StrictMode>,
);
