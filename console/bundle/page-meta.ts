// The page macro as the module takes it: Nuxt strips the call from a page as it builds, and the module's
// route carries the page's meta itself (`entry.ts`).
export function definePageMeta(_meta: Record<string, unknown>): void {}
