// Vite entry. Loads fonts, sets up base styles, mounts the SolidJS app.

import "@fontsource-variable/bricolage-grotesque";
import "@fontsource-variable/space-grotesk";
import "@fontsource/space-mono/400.css";
import "@fontsource/space-mono/700.css";

import { render } from "solid-js/web";
import App from "./App";
import "./styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Root element not found");
}
render(() => <App />, root);

// Dismiss the boot splash (index.html) now that the app shell has rendered —
// this is the moment the pre-render white screen would otherwise end. Two
// rAFs let the first real frame composite so the fade is smooth; a timeout is
// a belt-and-suspenders removal if `transitionend` never fires.
const splash = document.getElementById("boot-splash");
if (splash) {
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      splash.classList.add("boot-hidden");
      splash.addEventListener("transitionend", () => splash.remove(), {
        once: true,
      });
      setTimeout(() => splash.remove(), 800);
    }),
  );
}
