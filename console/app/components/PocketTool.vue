<template>
  <!-- One tool whole (docs/api.md, 「Tools」): what it is, what a session must hold for it, how the toolbook
       reaches it, its manual and its schemas. -->
  <div class="flex flex-col gap-4 p-4">
    <!-- The head: its mark, its names and the button that closes it, on one centre line. -->
    <div class="flex items-center gap-2">
      <UIcon :name="props.tool.icon ?? 'i-material-symbols-build-outline'" class="size-5 shrink-0 text-muted" />
      <div class="min-w-0 flex-1">
        <p class="font-mono text-sm break-all text-highlighted">{{ props.tool.namespace }}/{{ props.tool.name }}</p>
        <p v-if="props.tool.title" class="text-sm text-toned">{{ props.tool.title }}</p>
      </div>
      <UTooltip v-if="props.closable" :text="t('pocket.action.close')">
        <UButton icon="i-material-symbols-close" color="neutral" variant="ghost" :aria-label="t('pocket.action.close')" @click="emit('close')" />
      </UTooltip>
    </div>

    <p class="text-sm whitespace-pre-wrap text-default">{{ props.tool.description }}</p>

    <div class="flex flex-wrap gap-1">
      <UBadge :label="props.tool.reachedBy" color="neutral" variant="subtle" />
      <UBadge v-if="props.tool.hints.readOnly" :label="t('pocket.status.read_only')" color="success" variant="subtle" />
      <UBadge v-if="props.tool.hints.destructive" :label="t('pocket.status.destructive')" color="warning" variant="subtle" />
      <UBadge v-if="props.tool.system" :label="t('pocket.status.system')" color="info" variant="subtle" />
    </div>

    <PropertyList :rows="rows" />

    <section v-if="props.tool.manual" class="flex flex-col gap-1">
      <p class="text-sm font-medium text-muted">{{ t("pocket.field.manual") }}</p>
      <pre class="overflow-x-auto rounded-md bg-elevated p-3 text-sm whitespace-pre-wrap text-default">{{ props.tool.manual }}</pre>
    </section>

    <section class="flex flex-col gap-1">
      <p class="text-sm font-medium text-muted">{{ t("pocket.field.input") }}</p>
      <PocketSchema :schema="props.tool.input" />
    </section>

    <section v-if="props.tool.output" class="flex flex-col gap-1">
      <p class="text-sm font-medium text-muted">{{ t("pocket.field.output") }}</p>
      <PocketSchema :schema="props.tool.output" />
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";

import { stateWord } from "../domain/pocket-value";
import PocketSchema from "./PocketSchema.vue";

import type { Tool } from "../domain/pocket-api";
import type { BarProperty } from "bmo-ui/app/domain/tabs";

// Whether the tool closes itself here: beside the tool list, where no way back leads from it.
const props = defineProps<{ tool: Tool; closable: boolean }>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();

const none = computed(() => t("pocket.status.none"));

const rows = computed<BarProperty[]>(() => [
  { label: t("pocket.field.capabilities"), value: props.tool.capabilities.join(", ") || none.value, mono: true },
  { label: t("pocket.field.grant"), value: props.tool.grant ?? none.value, mono: true },
  { label: t("pocket.field.idempotent"), value: stateWord(t, props.tool.hints.idempotent) },
  { label: t("pocket.field.open_world"), value: stateWord(t, props.tool.hints.openWorld) },
  { label: t("pocket.field.listing"), value: props.tool.listing?.line ?? none.value }
]);
</script>
