<template>
  <!-- The catalogue (docs/api.md, 「Namespaces」 and 「Tools」): the namespaces beside the tools of the one chosen,
       and the tool opened beside them. On a phone each stands alone in turn. -->
  <div class="flex min-h-0 flex-1 flex-col md:flex-row">
    <nav
      v-if="!props.narrow || props.place.namespace === null"
      class="flex min-h-0 flex-col overflow-y-auto border-default md:w-56 md:shrink-0 md:border-e"
      :aria-label="t('pocket.label.namespaces')"
    >
      <PocketState :loading="namespaceList.loading.value" :failed="namespaceList.failed.value" :empty="namespaceList.items.value.length === 0" :empty-text="t('pocket.status.no_namespaces')">
        <button
          v-for="namespace in namespaceList.items.value"
          :key="namespace.name"
          type="button"
          class="flex items-center justify-between gap-2 px-4 py-2 text-start text-sm hover:bg-elevated/50"
          :class="namespace.name === props.place.namespace ? 'bg-elevated font-medium text-highlighted' : 'text-default'"
          @click="emit('go', { namespace: namespace.name, tool: null })"
        >
          <span class="truncate font-mono">{{ namespace.name }}</span>
          <UBadge :label="String(namespace.toolCount)" color="neutral" variant="subtle" size="sm" />
        </button>
      </PocketState>
    </nav>

    <section
      v-if="!props.narrow || (props.place.namespace !== null && props.place.tool === null)"
      class="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto"
      :aria-label="t('pocket.label.tools')"
    >
      <div v-if="props.narrow" class="border-b border-default px-2 py-1">
        <UButton icon="i-material-symbols-arrow-back" color="neutral" variant="ghost" :label="t('pocket.label.namespaces')" @click="emit('go', { namespace: null, tool: null })" />
      </div>
      <p v-if="props.place.namespace === null" class="p-6 text-center text-sm text-muted">{{ t("pocket.status.choose_namespace") }}</p>
      <PocketState v-else :loading="toolList.loading.value" :failed="toolList.failed.value" :empty="shownTools.length === 0" :empty-text="t(props.place.q === '' ? 'pocket.status.no_tools' : 'pocket.status.no_match')">
        <button
          v-for="tool in shownTools"
          :key="tool.name"
          type="button"
          class="flex flex-col gap-1 border-b border-default px-4 py-3 text-start hover:bg-elevated/50"
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
      class="flex min-h-0 flex-col overflow-y-auto border-default md:w-[28rem] md:shrink-0 md:border-s"
      :aria-label="t('pocket.label.tool')"
    >
      <PocketTool :tool="openTool" @close="emit('go', { tool: null })" />
    </aside>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, watch } from "vue";

import * as api from "../domain/pocket-api";
import { firstSentence, matches } from "../domain/pocket-place";
import { reading } from "../domain/pocket-read";
import PocketState from "./PocketState.vue";
import PocketTool from "./PocketTool.vue";

import type { Place } from "../domain/pocket-place";

const props = defineProps<{ place: Place; narrow: boolean; changes: number }>();
const emit = defineEmits<{ go: [place: Partial<Place>] }>();

const { t } = useI18n();

const namespaceList = reading(api.namespaces);
const toolList = reading(() => (props.place.namespace === null ? Promise.resolve([]) : api.tools(props.place.namespace)));

const shownTools = computed(() =>
  toolList.items.value.filter((tool) => matches(props.place.q, [tool.name, tool.title, tool.description, tool.grant, ...tool.capabilities]))
);
const openTool = computed(() => toolList.items.value.find((tool) => tool.name === props.place.tool) ?? null);

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
