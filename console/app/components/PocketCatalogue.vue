<template>
  <!-- The catalogue (docs/api.md, 「Namespaces」 and 「Tools」; docs/mcp-host.md, 「How the screen reads」): the
       namespaces beside the tools of the one chosen, and the tool opened beside them, in as many columns as the
       screen's own box has room for: three side by side, the open tool in the tool list's place, or each alone. -->
  <div ref="box" class="flex min-h-0 flex-1" :class="room === 'one' ? 'flex-col' : 'flex-row'" :data-room="room">
    <nav
      v-if="room !== 'one' || props.place.namespace === null"
      class="flex min-h-0 flex-col overflow-y-auto border-default"
      :class="room === 'one' ? '' : 'w-56 shrink-0 border-e'"
      :aria-label="t('pocket.label.namespaces')"
    >
      <PocketState :loading="namespaceList.loading.value" :failed="namespaceList.failed.value" :empty="namespaceList.items.value.length === 0" :empty-text="t('pocket.status.no_namespaces')">
        <button
          v-for="namespace in namespaceList.items.value"
          :key="namespace.name"
          type="button"
          class="flex items-center justify-between gap-2 px-4 py-2 text-start text-sm hover:bg-elevated/50 active:bg-elevated"
          :class="namespace.name === props.place.namespace ? 'bg-elevated font-medium text-highlighted' : 'text-default'"
          @click="emit('go', { namespace: namespace.name, tool: null })"
        >
          <span class="truncate font-mono">{{ namespace.name }}</span>
          <UBadge :label="namespace.toolCount.toLocaleString()" color="neutral" variant="subtle" size="sm" class="tabular-nums" />
        </button>
      </PocketState>
    </nav>

    <section
      v-if="listShown"
      class="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto"
      :class="room === 'three' && openTool !== null ? 'min-w-80' : ''"
      :aria-label="t('pocket.label.tools')"
    >
      <div v-if="room === 'one'" class="border-b border-default px-2 py-1">
        <UButton icon="i-material-symbols-arrow-back" color="neutral" variant="ghost" :label="t('pocket.label.namespaces')" @click="emit('go', { namespace: null, tool: null })" />
      </div>
      <p v-if="props.place.namespace === null" class="p-6 text-center text-sm text-muted">{{ t("pocket.status.choose_namespace") }}</p>
      <PocketState v-else :loading="toolList.loading.value" :failed="toolList.failed.value" :empty="shownTools.length === 0" :empty-text="t(props.place.q === '' ? 'pocket.status.no_tools' : 'pocket.status.no_match')">
        <button
          v-for="tool in shownTools"
          :key="tool.name"
          type="button"
          class="flex flex-col gap-1 border-b border-default px-4 py-3 text-start hover:bg-elevated/50 active:bg-elevated"
          :class="tool.name === props.place.tool ? 'bg-elevated' : ''"
          @click="emit('go', { tool: tool.name })"
        >
          <span class="flex flex-wrap items-center gap-1">
            <UIcon v-if="tool.icon" :name="tool.icon" class="size-4 text-muted" />
            <span class="font-mono text-sm text-highlighted">{{ tool.name }}</span>
            <UBadge :label="tool.reachedBy" color="neutral" variant="subtle" size="sm" />
            <UBadge v-if="tool.system" :label="t('pocket.status.system')" color="info" variant="subtle" size="sm" />
            <UBadge v-if="tool.grant" :label="tool.grant" color="warning" variant="outline" size="sm" />
          </span>
          <span class="text-sm text-toned">{{ firstSentence(tool.description) }}</span>
        </button>
      </PocketState>
    </section>

    <aside
      v-if="openTool !== null"
      class="flex min-h-0 min-w-0 flex-col overflow-y-auto border-default"
      :class="room === 'three' ? 'w-[28rem] shrink-0 border-s' : 'flex-1'"
      :aria-label="t('pocket.label.tool')"
    >
      <!-- Where the open tool stands in the tool list's place, the way back to the list stands over it. -->
      <div v-if="room !== 'three'" class="border-b border-default px-2 py-1">
        <UButton icon="i-material-symbols-arrow-back" color="neutral" variant="ghost" :label="t('pocket.label.tools')" @click="emit('go', { tool: null })" />
      </div>
      <PocketTool :tool="openTool" @close="emit('go', { tool: null })" />
    </aside>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, useTemplateRef, watch } from "vue";

import * as api from "../domain/pocket-api";
import { firstSentence, matches } from "../domain/pocket-place";
import { reading } from "../domain/pocket-read";
import { useRoom } from "../domain/pocket-room";
import PocketState from "./PocketState.vue";
import PocketTool from "./PocketTool.vue";

import type { Place } from "../domain/pocket-place";

const props = defineProps<{ place: Place; changes: number }>();
const emit = defineEmits<{ go: [place: Partial<Place>] }>();

const { t } = useI18n();

const namespaceList = reading(api.namespaces);
const toolList = reading(() => (props.place.namespace === null ? Promise.resolve([]) : api.tools(props.place.namespace)));

const shownTools = computed(() =>
  toolList.items.value.filter((tool) => matches(props.place.q, [tool.name, tool.title, tool.description, tool.grant, ...tool.capabilities]))
);
const openTool = computed(() => toolList.items.value.find((tool) => tool.name === props.place.tool) ?? null);

// How many columns the screen's own box holds, and whether the tool list stands: beside the open tool only where
// three fit, and alone at a phone's width only once a namespace is chosen.
const room = useRoom(useTemplateRef<HTMLElement>("box"));
const listShown = computed(() => {
  if (openTool.value !== null && room.value !== "three") return false;

  return room.value !== "one" || props.place.namespace !== null;
});

onMounted(() => {
  void namespaceList.read();
  void toolList.read();
});

watch(
  () => props.place.namespace,
  () => {
    toolList.items.value = [];
    void toolList.read();
  }
);

watch(
  () => props.changes,
  () => {
    void namespaceList.read();
    void toolList.read();
  }
);
</script>
