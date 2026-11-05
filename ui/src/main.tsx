import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Desktop from "./Desktop";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Desktop />
  </StrictMode>,
);
