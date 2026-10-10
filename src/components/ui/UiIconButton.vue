<script setup lang="ts">
import UiButton from './UiButton.vue'
import UiTooltip from './UiTooltip.vue'

// The tooltip root renders a fragment, so listeners and classes must be
// forwarded to the button explicitly.
defineOptions({ inheritAttrs: false })

withDefaults(defineProps<{
  label: string
  shortcut?: string
  size?: 'sm' | 'md'
  active?: boolean
  disabled?: boolean
  variant?: 'ghost' | 'secondary' | 'primary'
  side?: 'top' | 'bottom' | 'left' | 'right'
}>(), { size: 'md', variant: 'ghost' })
</script>

<template>
  <UiTooltip :label="label" :shortcut="shortcut" :side="side">
    <UiButton v-bind="$attrs" :variant="variant" :size="size" icon :active="active" :disabled="disabled" :aria-label="label"><slot /></UiButton>
  </UiTooltip>
</template>
