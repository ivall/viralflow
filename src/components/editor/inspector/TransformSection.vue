<script setup lang="ts">
import { ChevronLeft, ChevronRight, Diamond, FlipHorizontal2, FlipVertical2, RotateCcw, RotateCw } from '@lucide/vue'
import { computed } from 'vue'
import BezierEditor from './BezierEditor.vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiField from '@/components/ui/UiField.vue'
import UiIconButton from '@/components/ui/UiIconButton.vue'
import UiNumber from '@/components/ui/UiNumber.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import { keyframeAt, resolveTransform, DEFAULT_TRANSFORM } from '@/lib/engine/keyframes'
import type { BlendMode, Easing, TimelineItem, Transform } from '@/lib/types'
import { useEditor } from '@/stores/editor'

const props = defineProps<{ item: TimelineItem }>()
const editor = useEditor()

const elapsed = computed(() => editor.currentTime - props.item.start)
const inside = computed(() => elapsed.value >= -editor.keyframeTolerance && elapsed.value <= props.item.duration + editor.keyframeTolerance)
const state = computed(() => resolveTransform(props.item, elapsed.value))
const activeKeyframe = computed(() => props.item.keyframes?.length ? keyframeAt(props.item, elapsed.value, editor.keyframeTolerance) : null)
const animated = computed(() => Boolean(props.item.keyframes?.length))

const blendOptions: Array<{ value: BlendMode; label: string }> = [
  { value: 'normal', label: 'Normal' }, { value: 'multiply', label: 'Multiply' }, { value: 'screen', label: 'Screen' }, { value: 'overlay', label: 'Overlay' },
  { value: 'darken', label: 'Darken' }, { value: 'lighten', label: 'Lighten' }, { value: 'color-dodge', label: 'Color dodge' }, { value: 'color-burn', label: 'Color burn' },
  { value: 'hard-light', label: 'Hard light' }, { value: 'soft-light', label: 'Soft light' }, { value: 'difference', label: 'Difference' }, { value: 'exclusion', label: 'Exclusion' }
]
const easingOptions: Array<{ value: Easing; label: string }> = [
  { value: 'linear', label: 'Linear' }, { value: 'ease-in', label: 'Ease in' }, { value: 'ease-out', label: 'Ease out' }, { value: 'ease-in-out', label: 'Ease in-out' },
  { value: 'back-in', label: 'Back in' }, { value: 'back-out', label: 'Overshoot' }, { value: 'back-in-out', label: 'Back in-out' }, { value: 'custom', label: 'Custom curve' }
]

function set(patch: Partial<Transform>) {
  editor.setTransform(editor.selectedIds.map(id => ({ id, transform: patch })))
}

function reset() {
  editor.beginChange()
  editor.updateSelected({ transform: { ...DEFAULT_TRANSFORM }, keyframes: undefined, flipX: undefined, flipY: undefined }, false)
}
</script>

<template>
  <UiSection title="Transform">
    <template #actions>
      <UiIconButton label="Reset transform" size="sm" @click="reset"><RotateCcw class="size-3.5" /></UiIconButton>
    </template>
    <div class="grid grid-cols-2 gap-2">
      <UiNumber :model-value="state.x" label="X" suffix="%" :step="0.1" :precision="1" :scrub="0.1" @start="editor.beginChange()" @update:model-value="value => set({ x: value })" />
      <UiNumber :model-value="state.y" label="Y" suffix="%" :step="0.1" :precision="1" :scrub="0.1" @start="editor.beginChange()" @update:model-value="value => set({ y: value })" />
    </div>
    <UiSlider :model-value="state.scale * 100" label="Scale" :display="`${Math.round(state.scale * 100)}%`" :min="5" :max="400" :step="1" :center="100" @start="editor.beginChange()" @update:model-value="value => set({ scale: value / 100 })" />
    <div class="flex items-center gap-3">
      <span class="w-[84px] shrink-0 text-xs text-muted">Rotation</span>
      <UiNumber class="flex-1" :model-value="state.rotation" suffix="°" :step="1" :precision="1" :scrub="0.5" label="↻" @start="editor.beginChange()" @update:model-value="value => set({ rotation: value })" />
      <UiIconButton label="Rotate 90°" size="sm" @click="editor.beginChange(); set({ rotation: state.rotation + 90 })"><RotateCw class="size-3.5" /></UiIconButton>
      <UiIconButton label="Flip horizontally" size="sm" :active="item.flipX" @click="editor.updateSelected({ flipX: !item.flipX || undefined })"><FlipHorizontal2 class="size-3.5" /></UiIconButton>
      <UiIconButton label="Flip vertically" size="sm" :active="item.flipY" @click="editor.updateSelected({ flipY: !item.flipY || undefined })"><FlipVertical2 class="size-3.5" /></UiIconButton>
    </div>
    <UiSlider :model-value="state.opacity * 100" label="Opacity" :display="`${Math.round(state.opacity * 100)}%`" :min="0" :max="100" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ opacity: value / 100 })" />
    <UiField v-if="item.kind !== 'text'" label="Blend">
      <UiSelect :model-value="item.blend ?? 'normal'" :options="blendOptions" label="Blend mode" @update:model-value="value => editor.updateSelected({ blend: value === 'normal' ? undefined : value })" />
    </UiField>
  </UiSection>

  <UiSection title="Keyframes">
    <template #actions>
      <UiButton v-if="animated" variant="ghost" size="sm" @click="editor.clearKeyframes()">Clear all</UiButton>
    </template>
    <div class="flex items-center gap-1.5">
      <UiIconButton label="Previous keyframe" shortcut="Shift+K" size="sm" :disabled="!animated" @click="editor.jumpKeyframe(-1)"><ChevronLeft class="size-4" /></UiIconButton>
      <UiButton :variant="activeKeyframe ? 'primary' : 'outline'" size="sm" class="flex-1" :disabled="!inside" @click="editor.toggleKeyframeAtPlayhead()">
        <Diamond class="size-3.5" :class="activeKeyframe && 'fill-current'" />{{ activeKeyframe ? 'Remove keyframe' : 'Add keyframe' }}
      </UiButton>
      <UiIconButton label="Next keyframe" shortcut="Alt+K" size="sm" :disabled="!animated" @click="editor.jumpKeyframe(1)"><ChevronRight class="size-4" /></UiIconButton>
    </div>
    <UiField v-if="activeKeyframe" label="Easing" hint="Curve of the motion that arrives at this keyframe">
      <UiSelect :model-value="activeKeyframe.easing" :options="easingOptions" label="Keyframe easing" @update:model-value="value => editor.setKeyframeEasingAtPlayhead(value)" />
    </UiField>
    <BezierEditor v-if="activeKeyframe?.easing === 'custom' && activeKeyframe.bezier" :model-value="activeKeyframe.bezier" @start="editor.beginChange()" @update:model-value="value => editor.setKeyframeEasingAtPlayhead('custom', value, false)" />
    <p class="text-2xs leading-4 text-faint">{{ animated ? 'Changes at the playhead are saved into a keyframe.' : 'Add a keyframe, move the playhead and change the transform to animate.' }}</p>
  </UiSection>
</template>
