import React from "react";
import ReactDOM from "react-dom/client";
import "overlayscrollbars/overlayscrollbars.css";
// Bundled fonts (work offline): IBM Plex Sans for the interface and transcripts,
// IBM Plex Mono only for times, sizes and other figures.
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import App from "./App";

// Disable right-click context menu app-wide (native desktop app behaviour)
document.addEventListener("contextmenu", (e) => e.preventDefault());

const isOverlay = window.location.hash === "#overlay";
const previewView = import.meta.env.DEV && window.location.hash.startsWith("#preview/")
  ? window.location.hash.slice("#preview/".length)
  : null;

if (previewView) {
  // Dev-only design preview in a plain browser (see src/dev/Preview.tsx).
  import("./dev/Preview").then(({ Preview }) => {
    ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<Preview view={previewView} />);
  });
} else if (isOverlay) {
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
