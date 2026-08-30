import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { createControlRoomStore } from "./controlRoom";
import { registerPandoraWebMcpTools } from "./webmcp";
import "./styles.css";

export const controlRoom = createControlRoomStore();

void registerPandoraWebMcpTools(controlRoom).catch((error: unknown) => {
  console.warn("Pandora could not register WebMCP tools", error);
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App store={controlRoom} />
  </StrictMode>,
);
