<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(defineProps<{
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger' | 'outline'
  size?: 'sm' | 'md' | 'lg'
  icon?: boolean
  active?: boolean
  disabled?: boolean
  type?: 'button' | 'submit'
}>(), { variant: 'secondary', size: 'md', type: 'button' })

const classes = computed(() => [
  'inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap font-medium transition-colors disabled:pointer-events-none disabled:opacity-40',
  props.size === 'sm'
    ? (props.icon ? 'size-7 rounded-lg' : 'h-7 rounded-lg px-2.5 text-xs')
    : props.size === 'lg'
      ? (props.icon ? 'size-10 rounded-lg' : 'h-10 rounded-lg px-4 text-sm')
      : (props.icon ? 'size-8 rounded-lg' : 'h-8 rounded-lg px-3.5 text-xs'),
  {
    primary: 'bg-accent text-accent-ink hover:bg-accent-hover',
    secondary: 'bg-raised text-fg hover:bg-hover',
    ghost: props.active ? 'bg-hover text-fg' : 'text-muted hover:bg-hover hover:text-fg',
    danger: 'bg-danger/15 text-danger hover:bg-danger/25',
    outline: 'border border-line text-fg hover:border-line-strong hover:bg-raised'
  }[props.variant]
])
</script>

<template>
  <button :type="type" :class="classes" :disabled="disabled">
    <slot />
  </button>
</template>
