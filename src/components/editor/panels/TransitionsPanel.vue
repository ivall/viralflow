<script setup lang="ts">
import { computed, ref } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import { beginDrag } from '@/lib/dnd'
import { DEFAULT_TRANSITION, MAX_TRANSITION, MIN_TRANSITION, TRANSITIONS, trackSeams } from '@/lib/engine/transitions'
import type { TransitionKind } from '@/lib/types'
import { useEditor } from '@/stores/editor'
import { useToasts } from '@/stores/toast'

const editor = useEditor()
const duration = ref(DEFAULT_TRANSITION)
const seam = computed(() => {
  const item = editor.selectedItem
  const track = editor.selectedTrack
  if (!item || !track) return null
  const seams = trackSeams(track, editor.fps)
  return seams.find(candidate => candidate.to.id === item.id) ?? seams.find(candidate => candidate.from.id === item.id) ?? null
})

const glyphs: Record<string, string> = {
  fade: 'linear-gradient(90deg,#4f7cff,#8b6cf6)',
  black: 'linear-gradient(90deg,#4f7cff,#000 50%,#8b6cf6)',
  white: 'linear-gradient(90deg,#4f7cff,#fff 50%,#8b6cf6)',
  'slide-left': 'linear-gradient(90deg,#4f7cff 50%,#8b6cf6 50%)',
  'slide-right': 'linear-gradient(270deg,#4f7cff 50%,#8b6cf6 50%)',
  'slide-up': 'linear-gradient(180deg,#4f7cff 50%,#8b6cf6 50%)',
  'slide-down': 'linear-gradient(0deg,#4f7cff 50%,#8b6cf6 50%)',
  zoom: 'radial-gradient(circle,#8b6cf6 30%,#4f7cff 31%)',
  blur: 'linear-gradient(90deg,#4f7cff,#6b74f8,#8b6cf6)',
  'wipe-left': 'linear-gradient(90deg,#4f7cff 60%,#8b6cf6 60%)',
  'wipe-right': 'linear-gradient(90deg,#8b6cf6 40%,#4f7cff 40%)',
  flash: 'linear-gradient(90deg,#4f7cff,#fff,#8b6cf6)',
  spin: 'conic-gradient(#4f7cff 0 50%,#8b6cf6 0)'
}

function apply(kind: TransitionKind) {
  if (!seam.value) {
    useToasts().show('Select a clip that touches another clip', { detail: 'Or drag a transition onto the cut between two clips.' })
    return
  }
  editor.setTransition(seam.value.to.id, kind, duration.value)
}

function applyAll(kind: TransitionKind) {
  const track = editor.selectedTrack
  if (!track) return
  editor.setTransitionForTrack(track.id, kind, duration.value)
}

function onPointer(event: PointerEvent, kind: TransitionKind, label: string) {
  beginDrag(event, { type: 'transition', kind }, label, () => apply(kind))
}
</script>

<template>
  <div class="h-full overflow-y-auto p-3">
    <p class="mb-3 text-2xs leading-4 text-muted">Drag a transition onto a cut between two clips, or select a clip and click.</p>
    <div class="grid grid-cols-3 gap-2">
      <button
        v-for="preset in TRANSITIONS" :key="preset.kind"
        class="group flex flex-col gap-1 rounded-md p-1 text-left"
        :class="seam?.kind === preset.kind ? 'bg-accent/10' : 'hover:bg-hover'"
        @pointerdown="onPointer($event, preset.kind, preset.label)"
      >
        <span class="block aspect-video w-full rounded border-2" :class="seam?.kind === preset.kind ? 'border-accent' : 'border-transparent'" :style="{ background: glyphs[preset.kind] }" />
        <span class="truncate text-[10px] text-muted group-hover:text-fg">{{ preset.label }}</span>
      </button>
    </div>
    <div class="mt-4 space-y-2 border-t border-line pt-3">
      <UiSlider v-model="duration" label="Duration" :display="`${duration.toFixed(1)}s`" :min="MIN_TRANSITION" :max="MAX_TRANSITION" :step="0.1" />
      <div class="flex gap-2">
        <UiButton variant="outline" size="sm" class="flex-1" :disabled="!seam || seam.kind === 'none'" @click="apply('none')">Remove</UiButton>
        <UiButton variant="outline" size="sm" class="flex-1" :disabled="!editor.selectedTrack || !seam || seam.kind === 'none'" @click="applyAll(seam!.kind)">Apply to all cuts</UiButton>
      </div>
    </div>
  </div>
</template>
