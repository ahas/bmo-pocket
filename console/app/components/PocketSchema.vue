<template>
  <!-- A schema as its fields, one row each, a nested field under its parent (docs/mcp-host.md, 「How the screen
       reads」): its name, its type, whether it is required, what it is for and what it takes. A schema naming no
       fields stands as highlighted code. -->
  <ul v-if="fields.length > 0" class="flex flex-col divide-y divide-default rounded-md border border-default">
    <li v-for="(field, at) in fields" :key="at" class="flex flex-col gap-1 px-3 py-2" :style="{ paddingInlineStart: `${12 + field.depth * 16}px` }">
      <p class="flex flex-wrap items-center gap-2">
        <span class="font-mono text-sm break-all text-highlighted">{{ field.name }}</span>
        <UBadge v-if="field.type !== ''" :label="field.type" color="neutral" variant="subtle" size="sm" class="font-mono" />
        <UBadge v-if="field.required" :label="t('pocket.schema.required')" color="warning" variant="outline" size="sm" />
      </p>
      <p v-if="field.description !== ''" class="text-sm whitespace-pre-wrap text-toned">{{ field.description }}</p>
      <p v-for="said in field.takes" :key="said" class="text-sm break-words text-muted">{{ said }}</p>
    </li>
  </ul>
  <PocketCode v-else :text="code" />
</template>

<script setup lang="ts">
import { computed } from "vue";

import { schemaFields } from "../domain/pocket-value";
import PocketCode from "./PocketCode.vue";

const props = defineProps<{ schema: unknown }>();

const { t } = useI18n();

const fields = computed(() => schemaFields(t, props.schema));
const code = computed(() => JSON.stringify(props.schema, null, 2));
</script>
