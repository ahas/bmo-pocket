// What the entry imports that TypeScript reads no type of: the screen's page, and the stylesheet the build
// takes in.
declare module "*.vue" {
  import type { DefineComponent } from "vue";

  const component: DefineComponent;
  export default component;
}

declare module "*.css";
