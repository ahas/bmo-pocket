<template>
  <!-- What a view shows while its list is read, where the read failed, or where the list holds nothing. -->
  <ContentSkeleton v-if="props.loading" :rows="props.rows" list-class="flex flex-col gap-2 p-4" :label="t('pocket.status.loading')" />
  <p v-else-if="props.failed !== null" class="p-6 text-center text-sm text-error">
    {{ t("pocket.status.failed", { why: props.failed }) }}
  </p>
  <p v-else-if="props.empty" class="p-6 text-center text-sm text-muted">{{ props.emptyText }}</p>
  <slot v-else />
</template>

<script setup lang="ts">
const props = withDefaults(
  defineProps<{
    loading: boolean;
    failed: string | null;
    empty: boolean;
    emptyText: string;
    rows?: number;
  }>(),
  { rows: 4 }
);

const { t } = useI18n();
</script>
