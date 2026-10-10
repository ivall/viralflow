<script setup lang="ts">
import { computed } from 'vue'
import UiField from '@/components/ui/UiField.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSegmented from '@/components/ui/UiSegmented.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import type { Crop, Fit, TimelineItem } from '@/lib/types'
import { useEditor } from '@/stores/editor'

const props = defineProps<{ item: TimelineItem }>()
const editor = useEditor()
const crop = computed<Crop>(() => props.item.crop ?? { x: 0, y: 0, w: 1, h: 1 })
const fitOptions: Array<{ value: Fit; label: string; title: string }> = [
  { value: 'contain', label: 'Fit', title: 'Show the whole frame' },
  { value: 'cover', label: 'Fill', title: 'Fill the canvas, cropping edges' },
  { value: 'fill', label: 'Stretch', title: 'Stretch to the canvas' }
]

/** Crop edges in % (left, top, right, bottom). */
const edges = computed(() => ({
  left: crop.value.x * 100,
  top: crop.value.y * 100,
  right: (1 - crop.value.x - crop.value.w) * 100,
  bottom: (1 - crop.value.y - crop.value.h) * 100
}))

function setEdge(edge: 'left' | 'top' | 'right' | 'bottom', value: number) {
  const next = { ...edges.value, [edge]: value }
  const left = Math.min(next.left, 95 - next.right) / 100
  const top = Math.min(next.top, 95 - next.bottom) / 100
  const right = Math.min(next.right, 95 - next.left) / 100
  const bottom = Math.min(next.bottom, 95 - next.top) / 100
  editor.updateSelected({ crop: { x: left, y: top, w: 1 - left - right, h: 1 - top - bottom } }, false)
}

function resetCrop() {
  editor.updateSelected({ crop: undefined, focus: undefined })
}
</script>

<template>
  <UiSection title="Frame">
    <UiField label="Fit">
      <UiSegmented :model-value="item.fit ?? 'contain'" :options="fitOptions" label="Fit mode" @update:model-value="value => editor.updateSelected({ fit: value })" />
    </UiField>
    <template v-if="(item.fit ?? 'contain') === 'cover'">
      <UiSlider :model-value="item.focus?.x ?? 50" label="Focus X" :display="`${Math.round(item.focus?.x ?? 50)}%`" :min="0" :max="100" :step="1" :center="50" @start="editor.beginChange()" @update:model-value="value => editor.updateSelected({ focus: { x: value, y: item.focus?.y ?? 50 } }, false)" />
      <UiSlider :model-value="item.focus?.y ?? 50" label="Focus Y" :display="`${Math.round(item.focus?.y ?? 50)}%`" :min="0" :max="100" :step="1" :center="50" @start="editor.beginChange()" @update:model-value="value => editor.updateSelected({ focus: { x: item.focus?.x ?? 50, y: value } }, false)" />
    </template>
    <UiSlider :model-value="item.radius ?? 0" label="Corners" :display="`${Math.round(item.radius ?? 0)}`" :min="0" :max="400" :step="1" @start="editor.beginChange()" @update:model-value="value => editor.updateSelected({ radius: value }, false)" />
  </UiSection>
  <UiSection title="Crop" collapsible :default-open="Boolean(item.crop)">
    <template #actions>
      <button v-if="item.crop || item.focus" class="text-2xs text-muted hover:text-fg" @click="resetCrop">Reset</button>
    </template>
    <UiSlider :model-value="edges.left" label="Left" :display="`${Math.round(edges.left)}%`" :min="0" :max="90" :step="0.5" @start="editor.beginChange()" @update:model-value="value => setEdge('left', value)" />
    <UiSlider :model-value="edges.right" label="Right" :display="`${Math.round(edges.right)}%`" :min="0" :max="90" :step="0.5" @start="editor.beginChange()" @update:model-value="value => setEdge('right', value)" />
    <UiSlider :model-value="edges.top" label="Top" :display="`${Math.round(edges.top)}%`" :min="0" :max="90" :step="0.5" @start="editor.beginChange()" @update:model-value="value => setEdge('top', value)" />
    <UiSlider :model-value="edges.bottom" label="Bottom" :display="`${Math.round(edges.bottom)}%`" :min="0" :max="90" :step="0.5" @start="editor.beginChange()" @update:model-value="value => setEdge('bottom', value)" />
  </UiSection>
</template>
