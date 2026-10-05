// src/main.tsx — React entry point. Dark appearance only (design/design-system/README.md).

import React from "react";
import ReactDOM from "react-dom/client";

import "./theme/tokens.css";
import "./ds/components.css";
import "./app.css";
import App from "./App";

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error('Root element "#root" not found');

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
