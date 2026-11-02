<template>
  <!-- The external servers (docs/api.md, 「External servers」): each registration the host read, where its server
       is reached, and whether its tools are listed or why not. -->
  <div class="flex min-h-0 flex-1 flex-col overflow-y-auto">
    <PocketState :loading="list.loading.value" :failed="list.failed.value" :empty="shown.length === 0" :empty-text="t(props.q === '' ? 'pocket.status.no_servers' : 'pocket.status.no_match')">
      <article v-for="server in shown" :key="server.name" class="flex flex-col gap-2 border-b border-default px-4 py-3">
        <p class="flex flex-wrap items-center gap-2">
          <span class="font-mono text-sm text-highlighted">{{ server.name }}</span>
          <UBadge :label="server.transport" color="neutral" variant="subtle" size="sm" />
          <UBadge
            :label="server.listed === null ? t('pocket.status.not_listed') : t('pocket.status.listed', { count: server.listed })"
            :color="server.listed === null ? 'warning' : 'success'"
            variant="subtle"
            size="sm"
          />
        </p>
        <PropertyList :rows="rowsOf(server)" />
      </article>
    </PocketState>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, watch } from "vue";

import * as api from "../domain/pocket-api";
import { matches } from "../domain/pocket-place";
import { reading } from "../domain/pocket-read";
import PocketState from "./PocketState.vue";

import type { Server } from "../domain/pocket-api";
import type { BarProperty } from "bmo-ui/app/domain/tabs";

const props = defineProps<{ q: string; changes: number }>();

const { t } = useI18n();

const list = reading(api.servers);
const shown = computed(() => list.items.value.filter((server) => matches(props.q, [server.name, server.namespace, server.address])));

function rowsOf(server: Server): BarProperty[] {
  const rows: BarProperty[] = [
    { label: t("pocket.field.namespace"), value: server.namespace, mono: true },
    { label: t("pocket.field.address"), value: server.address, mono: true },
    { label: t("pocket.field.capabilities"), value: server.capabilities.join(", "), mono: true },
    { label: t("pocket.field.reached_by"), value: server.reachedBy }
  ];

  if (server.error !== null) rows.push({ label: t("pocket.field.error"), value: server.error });

  return rows;
}

onMounted(() => void list.read());
watch(() => props.changes, () => void list.read());
</script>
