// The host a browser test stands the pocket screen in: the console project as a layer, under the one app component
// a console wraps its screens in, so the screen meets the providers it meets in a console. A console renders a
// bundle's screen in the browser alone, so the host renders nothing on the server.
export default defineNuxtConfig({
  extends: ["../.."],
  devtools: { enabled: false },
  ssr: false
});
