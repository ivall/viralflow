<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'
import { ref } from 'vue'

const props = withDefaults(defineProps<{ title: string; collapsible?: boolean; defaultOpen?: boolean }>(), { collapsible: false, defaultOpen: true })
const open = ref(props.defaultOpen)
</script>

<template>
  <section class="border-b border-line px-4 py-3.5 last:border-b-0">
    <header class="flex h-5 items-center gap-2" :class="open && 'mb-3'">
      <button v-if="collapsible" class="flex items-center gap-1 text-xs font-semibold text-fg" :aria-expanded="open" @click="open = !open">
        <ChevronRight class="size-3.5 text-muted transition-transform" :class="open && 'rotate-90'" />{{ title }}
      </button>
      <h3 v-else class="text-xs font-semibold text-fg">{{ title }}</h3>
      <div class="ml-auto flex items-center gap-1"><slot name="actions" /></div>
    </header>
    <div v-show="open" class="space-y-2.5"><slot /></div>
  </section>
</template>
