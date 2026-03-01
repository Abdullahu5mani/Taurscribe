import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

const isOverlay = window.location.hash === "#overlay";

if (isOverlay) {
  import("./OverlayApp").then(({ OverlayApp }) => {
    ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
      <React.StrictMode><OverlayApp /></React.StrictMode>
    );
  });
} else {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode><App /></React.StrictMode>
  );
}
