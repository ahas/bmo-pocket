<template>
  <!-- The tool sessions standing (docs/api.md, 「A standing session」), in order of when each was issued: who holds
       each, what its record reaches and when it lapses. No token and no variable's value is ever read. -->
  <div class="flex min-h-0 flex-1 flex-col overflow-y-auto">
    <PocketState :loading="list.loading.value" :failed="list.failed.value" :empty="shown.length === 0" :empty-text="t(props.q === '' ? 'pocket.status.no_sessions' : 'pocket.status.no_match')">
      <article v-for="(session, index) in shown" :key="`${session.createdAt}/${index}`" class="flex flex-col gap-2 border-b border-default px-4 py-3">
        <p class="flex flex-wrap items-center gap-2">
          <span class="text-sm font-medium text-highlighted">{{ session.holderName }}</span>
          <UBadge v-for="namespace in session.namespaces" :key="namespace" :label="namespace" color="neutral" variant="subtle" size="sm" class="font-mono" />
        </p>
        <PropertyList :rows="rowsOf(session)" />
        <!-- The labels the issuer gave the session, one row each, the name beside its value. -->
        <section v-if="labelRows(session).length > 0" class="flex flex-col gap-1">
          <p class="text-sm font-medium text-muted">{{ t("pocket.field.labels") }}</p>
          <PropertyList :rows="labelRows(session)" />
        </section>
      </article>
    </PocketState>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, watch } from "vue";

import * as api from "../domain/pocket-api";
import { matches } from "../domain/pocket-place";
import { reading, stamp } from "../domain/pocket-read";
import { valueRows } from "../domain/pocket-value";
import PocketState from "./PocketState.vue";

import type { StandingSession } from "../domain/pocket-api";
import type { BarProperty } from "bmo-ui/app/domain/tabs";

const props = defineProps<{ q: string; changes: number }>();
const emit = defineEmits<{ read: [] }>();

const { t } = useI18n();

const list = reading(api.sessions);
const shown = computed(() =>
  list.items.value.filter((session) => matches(props.q, [session.holderName, ...session.namespaces, ...session.capabilities, ...valueRows(t, session.labels).flatMap((row) => [row.label, row.value])]))
);

function joined(values: readonly string[]): string {
  return values.length === 0 ? t("pocket.status.none") : values.join(", ");
}

function labelRows(session: StandingSession): BarProperty[] {
  return valueRows(t, session.labels);
}

function rowsOf(session: StandingSession): BarProperty[] {
  return [
    { label: t("pocket.field.capabilities"), value: joined(session.capabilities), mono: true },
    { label: t("pocket.field.grants"), value: joined(session.grants.map((grant) => grant.resource)), mono: true },
    { label: t("pocket.field.shown"), value: joined(session.shown), mono: true },
    { label: t("pocket.field.variables"), value: joined(session.variables), mono: true },
    { label: t("pocket.field.created_at"), value: stamp(session.createdAt) },
    { label: t("pocket.field.expires_at"), value: session.expiresAt === null ? t("pocket.status.forever") : stamp(session.expiresAt) }
  ];
}

// Each read of the list, said to the screen once it has landed.
async function read(): Promise<void> {
  await list.read();
  emit("read");
}

onMounted(() => void read());
watch(() => props.changes, () => void read());
</script>
