<script setup lang="ts" generic="T extends string | number">
import { Check, ChevronDown } from '@lucide/vue'
import { SelectContent, SelectItem, SelectItemIndicator, SelectItemText, SelectPortal, SelectRoot, SelectTrigger, SelectValue, SelectViewport } from 'reka-ui'
import { computed } from 'vue'

// SelectRoot renders no element, so classes go on the trigger.
defineOptions({ inheritAttrs: false })

const props = defineProps<{
  modelValue: T
  options: Array<{ value: T; label: string; hint?: string; style?: Record<string, string> }>
  label?: string
  size?: 'sm' | 'md'
}>()
const emit = defineEmits<{ 'update:modelValue': [value: T] }>()
const key = computed(() => String(props.modelValue))

function select(value: unknown) {
  const option = props.options.find(candidate => String(candidate.value) === String(value))
  if (option) emit('update:modelValue', option.value)
}
</script>

<template>
  <SelectRoot :model-value="key" @update:model-value="select">
    <SelectTrigger
      v-bind="$attrs"
      :aria-label="label"
      class="flex w-full min-w-0 items-center justify-between gap-2 rounded-md border border-line bg-app px-2.5 text-left text-xs text-fg outline-none transition-colors hover:border-line-strong focus-visible:border-accent data-[state=open]:border-accent"
      :class="size === 'sm' ? 'h-7' : 'h-[30px]'"
    >
      <SelectValue class="truncate" />
      <ChevronDown class="size-3.5 shrink-0 text-muted" />
    </SelectTrigger>
    <SelectPortal>
      <SelectContent position="popper" :side-offset="4" class="z-[200] max-h-[min(360px,var(--reka-select-content-available-height))] min-w-[var(--reka-select-trigger-width)] overflow-hidden rounded-lg border border-line bg-raised p-1 shadow-2xl">
        <SelectViewport>
          <SelectItem v-for="option in options" :key="String(option.value)" :value="String(option.value)" class="relative flex h-8 cursor-default items-center rounded-md pl-7 pr-3 text-xs text-fg outline-none data-[highlighted]:bg-hover">
            <SelectItemIndicator class="absolute left-2"><Check class="size-3.5 text-accent" /></SelectItemIndicator>
            <SelectItemText><span :style="option.style">{{ option.label }}</span></SelectItemText>
            <span v-if="option.hint" class="ml-auto pl-4 text-2xs text-faint">{{ option.hint }}</span>
          </SelectItem>
        </SelectViewport>
      </SelectContent>
    </SelectPortal>
  </SelectRoot>
</template>
