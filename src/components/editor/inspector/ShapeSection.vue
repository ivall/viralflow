<script setup lang="ts">
import { computed } from 'vue'
import UiColor from '@/components/ui/UiColor.vue'
import UiField from '@/components/ui/UiField.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import type { ShapeStyle, TimelineItem } from '@/lib/types'
import { useEditor } from '@/stores/editor'

const props = defineProps<{ item: TimelineItem }>()
const editor = useEditor()
const shape = computed<ShapeStyle>(() => props.item.shape ?? { kind: 'rect', width: 30, height: 20, color: '#ffffff', blur: 30 })

function set(patch: Partial<ShapeStyle>) {
  editor.updateSelected({ shape: { ...shape.value, ...patch } }, false)
}
</script>

<template>
  <UiSection title="Shape">
    <UiField v-if="shape.kind !== 'blur'" label="Color"><UiColor :model-value="shape.color" label="Shape color" @start="editor.beginChange()" @update:model-value="value => set({ color: value })" /></UiField>
    <UiSlider :model-value="shape.width" label="Width" :display="`${Math.round(shape.width)}%`" :min="1" :max="200" :step="0.5" @start="editor.beginChange()" @update:model-value="value => set(shape.kind === 'circle' && shape.width === shape.height ? { width: value, height: value } : { width: value })" />
    <UiSlider v-if="!(shape.kind === 'circle' && shape.width === shape.height)" :model-value="shape.height" label="Height" :display="`${shape.height.toFixed(1)}%`" :min="0.2" :max="200" :step="0.2" @start="editor.beginChange()" @update:model-value="value => set({ height: value })" />
    <UiSlider v-if="shape.kind === 'rect' || shape.kind === 'blur'" :model-value="item.radius ?? 0" label="Corners" :display="`${Math.round(item.radius ?? 0)}`" :min="0" :max="300" :step="1" @start="editor.beginChange()" @update:model-value="value => editor.updateSelected({ radius: value }, false)" />
    <UiSlider v-if="shape.kind === 'blur'" :model-value="shape.blur" label="Blur" :display="`${Math.round(shape.blur)}`" :min="2" :max="120" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ blur: value })" />
  </UiSection>
</template>
