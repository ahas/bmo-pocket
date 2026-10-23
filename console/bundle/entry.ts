import type { ConsoleBundleModule } from "bmo-ui/shared/console-manifest";

import "./style.css";

// The module's default export (bmo-ui's `docs/console-bundles.md`, "What a bundle is"): the tool host's screen,
// one route under its screen path, `/pocket`. Which view stands open, and which namespace and tool, is the
// address's query, so the screen is one page built once.
const bundle: ConsoleBundleModule = {
  routes: [
    {
      path: "/pocket",
      name: "pocket",
      component: () => import("../app/pages/pocket.vue")
    }
  ]
};

export default bundle;
