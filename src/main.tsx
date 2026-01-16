import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// Hide loading screen once React mounts
const hideLoadingScreen = () => {
  const loadingScreen = document.getElementById('loading-screen');
  if (loadingScreen) {
    loadingScreen.classList.add('hidden');
    setTimeout(() => loadingScreen.remove(), 300);
  }
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// Hide after a brief delay to ensure React has rendered
requestAnimationFrame(() => {
  requestAnimationFrame(hideLoadingScreen);
});
