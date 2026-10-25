import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/base.css";

const container = document.getElementById("root");
if (!container) throw new Error("DroidWin root element is missing");

createRoot(container).render(<App />);
