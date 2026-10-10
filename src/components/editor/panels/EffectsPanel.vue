<script setup lang="ts">
import { Check, Sparkles } from '@lucide/vue'
import { computed } from 'vue'
import { EFFECTS } from '@/lib/engine/effects'
import type { EffectKind } from '@/lib/types'
import { useEditor } from '@/stores/editor'
import { useToasts } from '@/stores/toast'

const editor = useEditor()
const targets = computed(() => editor.selectedItems.filter(item => item.kind === 'video' || item.kind === 'image' || item.kind === 'text'))
const current = computed(() => targets.value[0]?.effect ?? 'none')

function apply(kind: EffectKind) {
  if (!targets.value.length) {
    useToasts().show('Select a video, photo or text clip first')
    return
  }
  editor.updateSelected({ effect: kind === 'none' ? undefined : kind })
}
</script>

<template>
  <div class="h-full overflow-y-auto p-3">
    <p class="mb-3 text-2xs leading-4 text-muted">{{ targets.length ? `Applies to ${targets.length === 1 ? 'the selected clip' : `${targets.length} selected clips`}.` : 'Select a clip in the timeline to apply an effect.' }}</p>
    <div class="space-y-1">
      <button
        v-for="effect in EFFECTS" :key="effect.kind"
        class="flex w-full items-center gap-3 rounded-md border px-3 py-2 text-left transition-colors"
        :class="current === effect.kind && targets.length ? 'border-accent bg-accent/10' : 'border-transparent hover:bg-hover'"
        @click="apply(effect.kind)"
      >
        <Sparkles class="size-4 shrink-0" :class="effect.kind === 'none' ? 'text-faint' : 'text-accent'" />
        <span class="min-w-0 flex-1">
          <span class="block text-xs font-medium text-fg">{{ effect.label }}</span>
          <span class="block truncate text-[11px] text-muted">{{ effect.description }}</span>
        </span>
        <Check v-if="current === effect.kind && targets.length" class="size-4 text-accent" />
      </button>
    </div>
  </div>
</template>
