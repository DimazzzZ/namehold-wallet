import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { StartupGate } from "./components/StartupGate";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <StartupGate>
      <App />
    </StartupGate>
  </React.StrictMode>,
);
