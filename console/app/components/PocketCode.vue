<template>
  <!-- A long body that has no fields to stand as, in a block of its own with its highlighting (docs/mcp-host.md,
       「How the screen reads」). -->
  <div ref="root" class="markdown text-sm">
    <div class="markdown-code" :data-language="props.language">
      <pre><code>{{ props.text }}</code></pre>
    </div>
  </div>
</template>

<script setup lang="ts">
import { colourCode, forgetCode } from "bmo-ui/app/domain/code-colour";
import { useDarkScheme } from "bmo-ui/app/domain/scheme";
import { nextTick, onBeforeUnmount, onMounted, useTemplateRef, watch } from "vue";

const props = withDefaults(defineProps<{ text: string; language?: string }>(), { language: "json" });

const root = useTemplateRef<HTMLElement>("root");
const dark = useDarkScheme();

function colour(): void {
  if (root.value !== null) colourCode(root.value, dark.value);
}

onMounted(colour);
watch([() => props.text, dark], () => void nextTick(colour));
onBeforeUnmount(() => {
  if (root.value !== null) forgetCode(root.value);
});
</script>
