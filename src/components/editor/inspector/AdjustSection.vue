<script setup lang="ts">
import { RotateCcw } from '@lucide/vue'
import { computed } from 'vue'
import UiIconButton from '@/components/ui/UiIconButton.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import { NEUTRAL_ADJUST } from '@/lib/engine/color'
import type { Adjust, TimelineItem } from '@/lib/types'
import { useEditor } from '@/stores/editor'

const props = defineProps<{ item: TimelineItem }>()
const editor = useEditor()
const adjust = computed<Adjust>(() => ({ ...NEUTRAL_ADJUST, ...props.item.adjust }))
const changed = computed(() => Object.entries(adjust.value).some(([, value]) => Math.abs(value) > 1e-6))

const sliders: Array<{ key: keyof Adjust; label: string; min: number; max: number }> = [
  { key: 'exposure', label: 'Exposure', min: -1, max: 1 },
  { key: 'brightness', label: 'Brightness', min: -1, max: 1 },
  { key: 'contrast', label: 'Contrast', min: -1, max: 1 },
  { key: 'saturation', label: 'Saturation', min: -1, max: 1 },
  { key: 'temperature', label: 'Temperature', min: -1, max: 1 },
  { key: 'tint', label: 'Tint', min: -1, max: 1 },
  { key: 'hue', label: 'Hue', min: -1, max: 1 },
  { key: 'vignette', label: 'Vignette', min: 0, max: 1 }
]
</script>

<template>
  <UiSection title="Adjust">
    <template #actions>
      <UiIconButton v-if="changed" label="Reset adjustments" size="sm" @click="editor.updateSelected({ adjust: undefined })"><RotateCcw class="size-3.5" /></UiIconButton>
    </template>
    <UiSlider
      v-for="slider in sliders" :key="slider.key"
      :model-value="adjust[slider.key] * 100" :label="slider.label" :display="`${Math.round(adjust[slider.key] * 100)}`"
      :min="slider.min * 100" :max="slider.max * 100" :step="1" :center="0"
      @start="editor.beginChange()"
      @update:model-value="value => editor.updateSelected({ adjust: { ...adjust, [slider.key]: value / 100 } }, false)"
    />
  </UiSection>
</template>
