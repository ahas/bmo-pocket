<template>
  <!-- The tool host's screen (docs/api.md, 「The console face」): the catalogue, the schedules, the tool sessions
       standing and the external servers, each read as the issuer reads it. The view, the namespace, the tool and
       the search are the address. -->
  <UDashboardPanel id="pocket" :ui="{ body: 'p-0 sm:p-0 gap-0 sm:gap-0 overflow-y-hidden [&>*]:max-w-none' }">
    <template #header>
      <UDashboardNavbar :title="t('pocket.title.screen')" icon="i-material-symbols-home-repair-service-outline" />
    </template>

    <template #body>
      <div class="flex min-h-0 flex-1 flex-col">
        <ToolBar :label="t('pocket.label.toolbar')">
          <TabSwitch :items="viewItems" :model-value="place.view" :label="t('pocket.label.view')" :compact="narrow" @update:model-value="go({ view: $event, tool: null })" />
          <div>
            <UInput
              :model-value="place.q"
              icon="i-material-symbols-search"
              :placeholder="t('pocket.label.search')"
              :aria-label="t('pocket.label.search')"
              variant="none"
              class="w-40"
              @change="go({ q: ($event.target as HTMLInputElement).value })"
            />
          </div>
          <div class="ms-auto">
            <UButton icon="i-material-symbols-refresh" color="neutral" variant="ghost" :aria-label="t('pocket.action.refresh')" @click="changes += 1" />
          </div>
        </ToolBar>

        <PocketCatalogue v-if="place.view === 'catalogue'" :place="place" :narrow="narrow" :changes="changes" @go="go" />
        <PocketSchedules v-else-if="place.view === 'schedules'" :q="place.q" :changes="changes" />
        <PocketSessions v-else-if="place.view === 'sessions'" :q="place.q" :changes="changes" />
        <PocketServers v-else :q="place.q" :changes="changes" />
      </div>
    </template>
  </UDashboardPanel>
</template>

<script setup lang="ts">
import { useNarrow } from "bmo-ui/app/domain/screen";
import { computed, ref } from "vue";

import PocketCatalogue from "../components/PocketCatalogue.vue";
import PocketSchedules from "../components/PocketSchedules.vue";
import PocketServers from "../components/PocketServers.vue";
import PocketSessions from "../components/PocketSessions.vue";
import { placeOf, queryOf } from "../domain/pocket-place";

import type { Place, View } from "../domain/pocket-place";
import type { SwitchItem } from "bmo-ui/app/domain/tabs";

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const narrow = useNarrow();

// Bumped by the refresh button, so each view reads its list again.
const changes = ref(0);

const place = computed<Place>(() => placeOf(route.query));

const viewItems = computed<SwitchItem<View>[]>(() => [
  { value: "catalogue", label: t("pocket.view.catalogue"), icon: "i-material-symbols-category-outline" },
  { value: "schedules", label: t("pocket.view.schedules"), icon: "i-material-symbols-schedule-outline" },
  { value: "sessions", label: t("pocket.view.sessions"), icon: "i-material-symbols-key-outline" },
  { value: "servers", label: t("pocket.view.servers"), icon: "i-material-symbols-dns-outline" }
]);

function go(change: Partial<Place>): void {
  void router.push({ query: queryOf({ ...place.value, ...change }) });
}
</script>
