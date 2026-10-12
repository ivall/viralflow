<script setup lang="ts" generic="T extends string | number">
import type { Component } from 'vue'

defineProps<{ modelValue: T; options: Array<{ value: T; label?: string; icon?: Component; title?: string }>; label?: string }>()
const emit = defineEmits<{ 'update:modelValue': [value: T] }>()
</script>

<template>
  <div role="radiogroup" :aria-label="label" class="flex h-8 rounded-lg border border-line bg-app p-0.5">
    <button
      v-for="option in options" :key="String(option.value)"
      type="button" role="radio" :aria-checked="modelValue === option.value" :title="option.title ?? option.label"
      class="flex flex-1 items-center justify-center gap-1 rounded-md px-2 text-xs font-medium transition-colors"
      :class="modelValue === option.value ? 'bg-raised text-fg shadow-sm ring-1 ring-white/5' : 'text-muted hover:text-fg'"
      @click="emit('update:modelValue', option.value)"
    >
      <component :is="option.icon" v-if="option.icon" class="size-3.5" /><span v-if="option.label">{{ option.label }}</span>
    </button>
  </div>
</template>
