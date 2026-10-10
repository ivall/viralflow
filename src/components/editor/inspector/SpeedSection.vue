<script setup lang="ts">
import { computed } from 'vue'
import UiNumber from '@/components/ui/UiNumber.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import type { TimelineItem } from '@/lib/types'
import { formatDuration } from '@/services/media'
import { useEditor } from '@/stores/editor'

const props = defineProps<{ item: TimelineItem }>()
const editor = useEditor()
const speed = computed(() => props.item.speed || 1)
const presets = [0.25, 0.5, 1, 1.5, 2, 3, 5]

/** Logarithmic slider so 0.1× … 10× feels even. */
const sliderValue = computed(() => Math.log10(speed.value) * 100)

function setSpeed(value: number, record = true) {
  editor.updateSelected({ speed: Math.round(Math.min(10, Math.max(0.1, value)) * 100) / 100 }, record)
}
</script>

<template>
  <UiSection title="Speed">
    <div class="grid grid-cols-7 gap-1">
      <button
        v-for="preset in presets" :key="preset"
        class="h-7 rounded-md border text-2xs tabular-nums transition-colors"
        :class="Math.abs(speed - preset) < 1e-6 ? 'border-accent bg-accent/10 text-fg' : 'border-line text-muted hover:border-line-strong hover:text-fg'"
        @click="setSpeed(preset)"
      >{{ preset }}×</button>
    </div>
    <UiSlider :model-value="sliderValue" label="Speed" :display="`${speed.toFixed(2)}×`" :min="-100" :max="100" :step="1" :center="0" @start="editor.beginChange()" @update:model-value="value => setSpeed(Math.pow(10, value / 100), false)" />
    <div class="flex items-center gap-3">
      <span class="w-[84px] shrink-0 text-xs text-muted">Exact</span>
      <UiNumber class="flex-1" :model-value="speed" suffix="×" :min="0.1" :max="10" :step="0.05" :precision="2" @start="editor.beginChange()" @update:model-value="value => setSpeed(value, false)" />
    </div>
    <p class="text-2xs text-faint">Clip length: {{ formatDuration(item.duration) }}. Changing speed also changes the length; clips after it move only when they would overlap.</p>
  </UiSection>
</template>
