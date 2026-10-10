<script setup lang="ts">
defineProps<{ label: string; hint?: string }>()
</script>

<template>
  <div class="flex items-center gap-3">
    <span class="w-[84px] shrink-0 truncate text-xs text-muted" :title="hint">{{ label }}</span>
    <div class="min-w-0 flex-1"><slot /></div>
  </div>
</template>
