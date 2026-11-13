<template>
  <!-- The tool host's screen (docs/api.md, 「The console face」): the catalogue, the schedules, the tool sessions
       standing and the external servers, each read as the issuer reads it. The view, the namespace, the tool and
       the search are the address. -->
  <UDashboardPanel id="pocket" :ui="{ body: 'p-0 sm:p-0 gap-0 sm:gap-0 overflow-y-hidden [&>*]:max-w-none' }">
    <!-- Under a console that offers its title bar the screen draws none of its own, and either bar holds the
         screen's name and mark alone (docs/mcp-host.md, 「How the screen reads」). -->
    <template v-if="!barOffered" #header>
      <UDashboardNavbar :title="t('pocket.title.screen')" icon="i-material-symbols-home-repair-service-outline" />
    </template>

    <template #body>
      <div ref="box" class="flex min-h-0 flex-1 flex-col">
        <!-- The screen's own row and the only one under the bar: the way back at its left end, the reread at its
             right end, every button the layer's small one. -->
        <ToolBar :label="t('pocket.label.toolbar')">
          <div v-if="back !== null">
            <UTooltip :text="t(back.names)">
              <UButton icon="i-material-symbols-arrow-back" color="neutral" variant="ghost" :aria-label="t(back.names)" @click="go(back.to)" />
            </UTooltip>
          </div>
          <!-- More views than three are one dropdown naming the view standing (docs/mcp-host.md, 「How the screen reads」). -->
          <div>
            <USelect
              size="sm"
              variant="ghost"
              :model-value="place.view"
              :items="viewItems"
              :icon="standingIcon"
              :aria-label="t('pocket.label.view')"
              @update:model-value="go({ view: $event as View, tool: null })"
            />
          </div>
          <!-- The search gives up its width first, so the reread at the row's end stays in it at a phone's width. -->
          <div class="min-w-0 flex-1 shrink!">
            <UInput
              :model-value="place.q"
              icon="i-material-symbols-search"
              :placeholder="t('pocket.label.search')"
              :aria-label="t('pocket.label.search')"
              variant="none"
              class="w-full min-w-20 sm:max-w-60"
              @change="go({ q: ($event.target as HTMLInputElement).value })"
            />
          </div>
          <div class="ms-auto">
            <UTooltip :text="t('pocket.action.refresh')">
              <UButton icon="i-material-symbols-refresh" color="neutral" variant="ghost" :loading="rereading" :aria-label="t('pocket.action.refresh')" @click="reread" />
            </UTooltip>
          </div>
        </ToolBar>

        <PocketCatalogue v-if="place.view === 'catalogue'" :place="place" :room="room" :changes="changes" @go="go" @read="rereading = false" />
        <PocketSchedules v-else-if="place.view === 'schedules'" :q="place.q" :changes="changes" @read="rereading = false" />
        <PocketSessions v-else-if="place.view === 'sessions'" :q="place.q" :changes="changes" @read="rereading = false" />
        <PocketServers v-else :q="place.q" :changes="changes" @read="rereading = false" />
      </div>
    </template>
  </UDashboardPanel>
</template>

<script setup lang="ts">
import { computed, inject, ref, useTemplateRef } from "vue";

import PocketCatalogue from "../components/PocketCatalogue.vue";
import PocketSchedules from "../components/PocketSchedules.vue";
import PocketServers from "../components/PocketServers.vue";
import PocketSessions from "../components/PocketSessions.vue";
import { placeOf, queryOf } from "../domain/pocket-place";
import { useRoom, wayBack } from "../domain/pocket-room";

import type { Place, View } from "../domain/pocket-place";

interface ViewItem {
  value: View;
  label: string;
  icon: string;
}

const route = useRoute();
const router = useRouter();
const { t } = useI18n();

// Whether the console this screen stands in offers its own title bar, under bmo-web's key for it.
const barOffered = inject("bmo-console-title-bar", null) !== null;

// Bumped by the refresh button, so each view reads its list again; the button spins until the view says its
// lists are read.
const changes = ref(0);
const rereading = ref(false);

function reread(): void {
  rereading.value = true;
  changes.value += 1;
}

const place = computed<Place>(() => placeOf(route.query));

// The room the box under the toolbar has, which the catalogue's columns and the way back are read by.
const room = useRoom(useTemplateRef<HTMLElement>("box"));
const back = computed(() => wayBack(place.value, room.value));

const viewItems = computed<ViewItem[]>(() => [
  { value: "catalogue", label: t("pocket.view.catalogue"), icon: "i-material-symbols-category-outline" },
  { value: "schedules", label: t("pocket.view.schedules"), icon: "i-material-symbols-schedule-outline" },
  { value: "sessions", label: t("pocket.view.sessions"), icon: "i-material-symbols-key-outline" },
  { value: "servers", label: t("pocket.view.servers"), icon: "i-material-symbols-dns-outline" }
]);
// The dropdown names the standing view by its icon as well as its word.
const standingIcon = computed(() => viewItems.value.find((item) => item.value === place.value.view)?.icon);

function go(change: Partial<Place>): void {
  void router.push({ query: queryOf({ ...place.value, ...change }) });
}
</script>
