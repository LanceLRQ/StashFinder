import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import Assets from "./Assets";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Assets />
  </StrictMode>,
);
