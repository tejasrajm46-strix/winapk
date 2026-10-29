import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { OverlayWindow } from "./components/OverlayWindows";
import "./styles/base.css";
import "./styles/overlay.css";

const container = document.getElementById("root");
if (!container) throw new Error("DroidWin root element is missing");

/**
 * One HTML entry point, three windows. The label the Rust side gave this webview
 * decides what it draws: the manager screen, or one of the two overlays. Outside
 * Tauri (a browser during development) the label is unknown and the manager
 * renders, which is what the screens expect.
 */
function windowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return "main";
  }
}

const label = windowLabel();
const overlay = label.startsWith("overlay-");
if (overlay) document.documentElement.classList.add("overlay-root");

createRoot(container).render(
  overlay ? <OverlayWindow label={label} /> : <App />,
);
