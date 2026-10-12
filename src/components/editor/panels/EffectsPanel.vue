<script setup lang="ts">
import { Ban } from '@lucide/vue'
import { computed } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import { beginDrag } from '@/lib/dnd'
import { EFFECTS } from '@/lib/engine/effects'
import type { EffectKind } from '@/lib/types'
import { useEditor } from '@/stores/editor'
import { useToasts } from '@/stores/toast'

const editor = useEditor()
const targets = computed(() => editor.selectedItems.filter(item => item.kind === 'video' || item.kind === 'image' || item.kind === 'text'))
const current = computed(() => targets.value[0]?.effect ?? 'none')
const presets = EFFECTS.filter(effect => effect.kind !== 'none')

function apply(kind: EffectKind) {
  if (!targets.value.length) {
    useToasts().show('Select a video, photo or text clip first')
    return
  }
  editor.setEffect(targets.value.map(item => item.id), kind)
}

function onPointer(event: PointerEvent, kind: EffectKind, label: string) {
  beginDrag(event, { type: 'effect', kind }, label, () => apply(kind))
}
</script>

<template>
  <div class="h-full overflow-y-auto p-3">
    <p class="mb-3 text-2xs leading-4 text-muted">
      Drag an effect onto a clip{{ targets.length ? `, or click to apply it to ${targets.length === 1 ? 'the selected clip' : `${targets.length} selected clips`}` : '' }}. Move it or drag its edges on the timeline to choose when it plays.
    </p>
    <div class="grid grid-cols-3 gap-2">
      <button
        v-for="effect in presets" :key="effect.kind"
        class="group flex flex-col gap-1 rounded-md p-1 text-left"
        :class="targets.length && current === effect.kind ? 'bg-accent/10' : 'hover:bg-hover'"
        :title="effect.description"
        @pointerdown="onPointer($event, effect.kind, effect.label)"
      >
        <span class="glyph block aspect-video w-full overflow-hidden rounded border-2" :class="targets.length && current === effect.kind ? 'border-accent' : 'border-transparent'" :data-effect="effect.kind">
          <span class="scene"><span class="sun" /><span class="hill" /></span>
        </span>
        <span class="truncate text-[11px] text-muted group-hover:text-fg">{{ effect.label }}</span>
      </button>
    </div>
    <div class="mt-4 border-t border-line pt-3">
      <UiButton variant="outline" size="sm" class="w-full" :disabled="!targets.length || current === 'none'" @click="apply('none')"><Ban class="size-3.5" />Remove effect</UiButton>
    </div>
  </div>
</template>

<style scoped>
/* A tiny landscape that each effect treats the way it treats a clip. */
.glyph {
  position: relative;
  background: #1a1d29;
}

.scene {
  position: absolute;
  inset: 0;
  background: linear-gradient(160deg, #4f7cff, #8b6cf6);
  transform-origin: 50% 60%;
}

.sun {
  position: absolute;
  left: 58%;
  top: 22%;
  width: 26%;
  aspect-ratio: 1;
  border-radius: 50%;
  background: #ffd166;
}

.hill {
  position: absolute;
  left: -10%;
  right: -10%;
  bottom: -38%;
  height: 70%;
  border-radius: 50%;
  background: #2b2f6b;
}

.glyph::after {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
}

/* Static looks, so the grid reads without hovering. */
[data-effect='punch']::after { background: repeating-conic-gradient(from 0deg at 50% 50%, rgb(255 255 255 / 0.22) 0 4deg, transparent 4deg 24deg); mask: radial-gradient(circle, transparent 30%, #000 60%); }
[data-effect='shake'] .sun { box-shadow: -7px 0 0 rgb(255 209 102 / 0.45), 7px 0 0 rgb(255 209 102 / 0.2); }
[data-effect='zoom-in'] .scene { transform: scale(1.12); }
[data-effect='zoom-out'] .scene { transform: scale(0.84); border-radius: 2px; }
[data-effect='blur'] .scene { filter: blur(2.5px); transform: scale(1.1); }
[data-effect='vhs'] .scene { filter: saturate(0.6) contrast(0.9) sepia(0.25); }
[data-effect='vhs']::after { background: repeating-linear-gradient(0deg, rgb(0 0 0 / 0.28) 0 1px, transparent 1px 3px); }
[data-effect='flash']::after { background: radial-gradient(circle at 50% 50%, rgb(255 255 255 / 0.75), rgb(255 255 255 / 0.15) 70%); }
[data-effect='glitch'] .scene { clip-path: polygon(0 0, 100% 0, 100% 34%, 0 34%, 0 46%, 100% 46%, 100% 100%, 0 100%); }
[data-effect='glitch']::after { background: linear-gradient(180deg, transparent 34%, rgb(255 60 120 / 0.55) 34% 46%, transparent 46%); transform: translateX(8%); }

/* Motion plays while hovering. */
@media (prefers-reduced-motion: no-preference) {
  .group:hover [data-effect='punch'] .scene { animation: punch 1.1s ease-out infinite; }
  .group:hover [data-effect='shake'] .scene { animation: shake 1.1s linear infinite; }
  .group:hover [data-effect='zoom-in'] .scene { animation: push 1.8s ease-in-out infinite alternate; }
  .group:hover [data-effect='zoom-out'] .scene { animation: pull 1.8s ease-in-out infinite alternate; }
  .group:hover [data-effect='flash']::after { animation: flash 1.1s ease-out infinite; }
  .group:hover [data-effect='blur'] .scene { animation: soften 1.8s ease-in-out infinite alternate; }
  .group:hover [data-effect='vhs']::after { animation: roll 0.6s linear infinite; }
  .group:hover [data-effect='glitch']::after { animation: glitch 0.5s steps(2) infinite; }
}

@keyframes punch {
  0% { transform: scale(1.18); }
  35%, 100% { transform: scale(1); }
}

@keyframes shake {
  0%, 100% { transform: translate(0, 0) scale(1.08); }
  10% { transform: translate(-6%, 3%) scale(1.08); }
  20% { transform: translate(5%, -4%) scale(1.08); }
  30% { transform: translate(-4%, -2%) scale(1.08); }
  40% { transform: translate(3%, 3%) scale(1.08); }
  50% { transform: translate(-1%, 1%) scale(1.08); }
  60% { transform: translate(0, 0) scale(1.08); }
}

@keyframes push {
  from { transform: scale(1); }
  to { transform: scale(1.22); }
}

@keyframes pull {
  from { transform: scale(1.22); }
  to { transform: scale(1); }
}

@keyframes flash {
  0% { opacity: 1; }
  45%, 100% { opacity: 0; }
}

@keyframes soften {
  from { filter: blur(0); }
  to { filter: blur(3px); }
}

@keyframes roll {
  from { background-position: 0 0; }
  to { background-position: 0 3px; }
}

@keyframes glitch {
  0% { transform: translateX(8%); }
  50% { transform: translateX(-10%); }
  100% { transform: translateX(8%); }
}
</style>
