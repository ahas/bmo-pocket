// The tool host's screen as a Nuxt layer on bmo-ui, which the typecheck and the module's build read: Nuxt's
// generated declarations name what the screen imports by itself, and the module's build takes each of those from
// the package a console shares (bundle/vite.config.ts).
export default defineNuxtConfig({
  extends: ["bmo-ui"],
  modules: ["@nuxtjs/i18n"],
  i18n: {
    strategy: "no_prefix",
    defaultLocale: "ko",
    locales: [
      { code: "ko", language: "ko-KR", file: "ko.json" },
      { code: "en", language: "en-US", file: "en.json" }
    ]
  },
  compatibilityDate: "2025-09-01",
  devtools: { enabled: false }
});
