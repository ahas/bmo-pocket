<template>
  <!-- The schedules (docs/api.md, 「Schedules」): every call the host makes by itself, in order of when each is
       next due, and where each stands. -->
  <div class="flex min-h-0 flex-1 flex-col overflow-y-auto">
    <PocketState :loading="list.loading.value" :failed="list.failed.value" :empty="shown.length === 0" :empty-text="t(props.q === '' ? 'pocket.status.no_schedules' : 'pocket.status.no_match')">
      <article v-for="(schedule, index) in shown" :key="`${schedule.namespace}/${schedule.toolName}/${index}`" class="flex flex-col gap-2 border-b border-default px-4 py-3">
        <p class="flex flex-wrap items-center gap-2">
          <span class="font-mono text-sm text-highlighted">{{ schedule.namespace }}/{{ schedule.toolName }}</span>
          <UBadge :label="schedule.cron" color="neutral" variant="subtle" size="sm" class="font-mono" />
          <UBadge v-if="schedule.running" :label="t('pocket.status.running')" color="info" variant="subtle" size="sm" />
          <UBadge v-if="schedule.lastError !== null" :label="t('pocket.status.failing')" color="error" variant="subtle" size="sm" />
        </p>
        <PropertyList :rows="rowsOf(schedule)" />
      </article>
    </PocketState>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, watch } from "vue";

import * as api from "../domain/pocket-api";
import { matches } from "../domain/pocket-place";
import { reading, stamp } from "../domain/pocket-read";
import PocketState from "./PocketState.vue";

import type { Schedule } from "../domain/pocket-api";
import type { BarProperty } from "bmo-ui/app/domain/tabs";

const props = defineProps<{ q: string; changes: number }>();

const { t } = useI18n();

const list = reading(api.schedules);
const shown = computed(() => list.items.value.filter((schedule) => matches(props.q, [schedule.namespace, schedule.toolName, schedule.cron])));

function rowsOf(schedule: Schedule): BarProperty[] {
  const rows: BarProperty[] = [
    { label: t("pocket.field.next_at"), value: stamp(schedule.nextAt) },
    { label: t("pocket.field.last_at"), value: schedule.lastAt === null ? t("pocket.status.never") : stamp(schedule.lastAt) },
    { label: t("pocket.field.arguments"), value: JSON.stringify(schedule.arguments), mono: true }
  ];

  if (schedule.lastError !== null) rows.push({ label: t("pocket.field.last_error"), value: schedule.lastError });

  return rows;
}

onMounted(() => void list.read());
watch(() => props.changes, () => void list.read());
</script>
