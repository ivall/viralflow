<script setup lang="ts">
import { SliderRange, SliderRoot, SliderThumb, SliderTrack } from 'reka-ui'
import { computed } from 'vue'

const props = withDefaults(defineProps<{
  modelValue: number
  min?: number
  max?: number
  step?: number
  label?: string
  display?: string
  /** Value marked with a tick (e.g. 0 on a bipolar slider). Double-click resets to it. */
  center?: number
}>(), { min: 0, max: 1, step: 0.01 })
const emit = defineEmits<{ 'update:modelValue': [value: number]; start: [] }>()
const value = computed(() => [Math.min(props.max, Math.max(props.min, props.modelValue))])
const centerPercent = computed(() => props.center === undefined ? null : ((props.center - props.min) / (props.max - props.min)) * 100)

function reset() {
  if (props.center === undefined) return
  emit('start')
  emit('update:modelValue', props.center)
}
</script>

<template>
  <div class="flex items-center gap-3">
    <span v-if="label" class="w-[84px] shrink-0 truncate text-xs text-muted" @dblclick="reset">{{ label }}</span>
    <SliderRoot
      class="relative flex h-5 flex-1 touch-none select-none items-center"
      :model-value="value" :min="min" :max="max" :step="step"
      @pointerdown="emit('start')"
      @keydown="emit('start')"
      @dblclick="reset"
      @update:model-value="values => values && emit('update:modelValue', values[0])"
    >
      <SliderTrack class="relative h-[3px] flex-1 overflow-hidden rounded-full bg-line">
        <SliderRange class="absolute h-full rounded-full bg-accent" />
      </SliderTrack>
      <span v-if="centerPercent !== null" class="pointer-events-none absolute top-1/2 h-2 w-px -translate-y-1/2 bg-line-strong" :style="{ left: `${centerPercent}%` }" />
      <SliderThumb class="block size-3 rounded-full border-2 border-accent bg-panel shadow transition-transform hover:scale-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50" :aria-label="label" />
    </SliderRoot>
    <span v-if="display !== undefined" class="w-11 shrink-0 text-right text-xs tabular-nums text-fg">{{ display }}</span>
  </div>
</template>
