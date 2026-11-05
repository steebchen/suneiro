import { invoke } from "@tauri-apps/api/core";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { installErrorReporting } from "./lib/errorReports";
import "./styles.css";

const boot = () => createRoot(document.getElementById("root")!).render(<App />);

// In a plain browser during development, fake the backend (see dev/mock.ts).
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  void import("./dev/mock").then(boot);
} else {
  installErrorReporting((error) => invoke("report_error", { error }));
  boot();
}
