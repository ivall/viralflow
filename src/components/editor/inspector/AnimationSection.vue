<script setup lang="ts">
import { Play, Trash2 } from '@lucide/vue'
import { computed } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiField from '@/components/ui/UiField.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import { ANIMATIONS_IN, ANIMATIONS_OUT, animationWindows, motionSpan } from '@/lib/engine/animation'
import { EFFECTS } from '@/lib/engine/effects'
import { MAX_TRANSITION, MIN_TRANSITION, TRANSITIONS, trackSeams } from '@/lib/engine/transitions'
import type { AnimationInKind, AnimationOutKind, EffectKind, TimelineItem, TransitionKind } from '@/lib/types'
import { useEditor } from '@/stores/editor'

const props = defineProps<{ item: TimelineItem }>()
const editor = useEditor()
// A word-highlight caption is one clip per word but animates as one text.
const span = computed(() => motionSpan(props.item, editor.selectedTrack?.items ?? []))
const windows = computed(() => animationWindows(span.value))
const seam = computed(() => {
  const track = editor.selectedTrack
  return track ? trackSeams(track, editor.fps).find(candidate => candidate.to.id === props.item.id) ?? null : null
})
const transitionOptions = computed(() => [{ value: 'none' as TransitionKind, label: 'None' }, ...TRANSITIONS.map(preset => ({ value: preset.kind as TransitionKind, label: preset.label }))])
const effectOptions = EFFECTS.map(effect => ({ value: effect.kind, label: effect.label }))
const maxTransition = computed(() => seam.value ? Math.min(MAX_TRANSITION, seam.value.from.duration, seam.value.to.duration) : MAX_TRANSITION)

function setIn(kind: AnimationInKind) {
  editor.setAnimation('in', kind)
  preview('in')
}

function setOut(kind: AnimationOutKind) {
  editor.setAnimation('out', kind)
  preview('out')
}

/** Plays the animated part of the clip once so the user sees the result. */
function preview(slot: 'in' | 'out') {
  const { start, duration } = span.value
  const length = slot === 'in' ? windows.value.inDuration : windows.value.outDuration
  if (!length) return
  const from = slot === 'in' ? start : start + duration - length - 0.2
  editor.seek(Math.max(0, from))
  editor.play()
  setTimeout(() => { if (editor.playing) editor.stop() }, (length + 0.35) * 1000)
}
</script>

<template>
  <UiSection title="In">
    <template #actions>
      <UiButton v-if="windows.inKind !== 'none'" variant="ghost" size="sm" @click="preview('in')"><Play class="size-3" />Preview</UiButton>
    </template>
    <div class="grid grid-cols-3 gap-1.5">
      <button
        v-for="preset in ANIMATIONS_IN" :key="preset.kind"
        class="h-8 truncate rounded-md border px-1.5 text-2xs transition-colors"
        :class="windows.inKind === preset.kind ? 'border-accent bg-accent/10 text-fg' : 'border-line text-muted hover:border-line-strong hover:text-fg'"
        @click="setIn(preset.kind)"
      >{{ preset.label }}</button>
    </div>
    <UiSlider v-if="windows.inKind !== 'none'" :model-value="span.animIn!.duration" label="Duration" :display="`${span.animIn!.duration.toFixed(2)}s`" :min="0.1" :max="Math.max(0.2, Math.min(5, span.duration))" :step="0.05" @start="editor.beginChange()" @update:model-value="value => editor.setAnimationDuration('in', value)" />
  </UiSection>

  <UiSection title="Out">
    <template #actions>
      <UiButton v-if="windows.outKind !== 'none'" variant="ghost" size="sm" @click="preview('out')"><Play class="size-3" />Preview</UiButton>
    </template>
    <div class="grid grid-cols-3 gap-1.5">
      <button
        v-for="preset in ANIMATIONS_OUT" :key="preset.kind"
        class="h-8 truncate rounded-md border px-1.5 text-2xs transition-colors"
        :class="windows.outKind === preset.kind ? 'border-accent bg-accent/10 text-fg' : 'border-line text-muted hover:border-line-strong hover:text-fg'"
        @click="setOut(preset.kind)"
      >{{ preset.label }}</button>
    </div>
    <UiSlider v-if="windows.outKind !== 'none'" :model-value="span.animOut!.duration" label="Duration" :display="`${span.animOut!.duration.toFixed(2)}s`" :min="0.1" :max="Math.max(0.2, Math.min(5, span.duration))" :step="0.05" @start="editor.beginChange()" @update:model-value="value => editor.setAnimationDuration('out', value)" />
  </UiSection>

  <UiSection v-if="item.kind !== 'shape'" title="Effect">
    <UiField label="Effect">
      <UiSelect :model-value="item.effect ?? 'none'" :options="effectOptions" label="Clip effect" @update:model-value="value => editor.updateSelected({ effect: value === 'none' ? undefined : value as EffectKind })" />
    </UiField>
  </UiSection>

  <UiSection v-if="seam" title="Transition in">
    <UiField label="Type">
      <UiSelect :model-value="seam.kind" :options="transitionOptions" label="Transition" @update:model-value="value => editor.setTransition(item.id, value)" />
    </UiField>
    <template v-if="seam.kind !== 'none'">
      <UiSlider :model-value="seam.duration" label="Duration" :display="`${seam.duration.toFixed(2)}s`" :min="MIN_TRANSITION" :max="maxTransition" :step="0.05" @start="editor.beginChange()" @update:model-value="value => editor.setTransition(item.id, seam!.kind, value, false)" />
      <UiButton variant="ghost" size="sm" class="w-full" @click="editor.setTransition(item.id, 'none')"><Trash2 class="size-3.5" />Remove transition</UiButton>
    </template>
  </UiSection>
</template>
