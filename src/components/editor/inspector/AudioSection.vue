<script setup lang="ts">
import { Captions, VolumeX } from '@lucide/vue'
import { computed } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import type { TimelineItem } from '@/lib/types'
import { useEditor } from '@/stores/editor'

const props = defineProps<{ item: TimelineItem }>()
const emit = defineEmits<{ captions: [] }>()
const editor = useEditor()
const asset = computed(() => props.item.assetId ? editor.assetsById.get(props.item.assetId) : undefined)
const silent = computed(() => props.item.kind === 'video' && asset.value?.hasAudio === false)
const volume = computed(() => props.item.volume ?? 1)
const maxFade = computed(() => Math.max(0.1, Math.min(10, props.item.duration / 2)))

function decibels(value: number) {
  if (value <= 0.0001) return '−∞ dB'
  const db = 20 * Math.log10(value)
  return `${db > 0 ? '+' : ''}${db.toFixed(1)} dB`
}
</script>

<template>
  <UiSection title="Audio">
    <p v-if="silent" class="text-xs text-muted">This video has no sound.</p>
    <template v-else>
      <UiSlider :model-value="volume * 100" label="Volume" :display="`${Math.round(volume * 100)}%`" :min="0" :max="200" :step="1" :center="100" @start="editor.beginChange()" @update:model-value="value => editor.updateSelected({ volume: value / 100 }, false)" />
      <p class="-mt-1 pl-[96px] text-[10px] text-faint">{{ decibels(volume) }}</p>
      <UiSlider :model-value="item.fadeIn ?? 0" label="Fade in" :display="`${(item.fadeIn ?? 0).toFixed(1)}s`" :min="0" :max="maxFade" :step="0.1" @start="editor.beginChange()" @update:model-value="value => editor.updateSelected({ fadeIn: value }, false)" />
      <UiSlider :model-value="item.fadeOut ?? 0" label="Fade out" :display="`${(item.fadeOut ?? 0).toFixed(1)}s`" :min="0" :max="maxFade" :step="0.1" @start="editor.beginChange()" @update:model-value="value => editor.updateSelected({ fadeOut: value }, false)" />
      <div class="flex gap-2 pt-1">
        <UiButton :variant="item.muted ? 'primary' : 'outline'" size="sm" class="flex-1" @click="editor.toggleItemFlag('muted')"><VolumeX class="size-3.5" />{{ item.muted ? 'Muted' : 'Mute' }}</UiButton>
        <UiButton variant="outline" size="sm" class="flex-1" @click="emit('captions')"><Captions class="size-3.5" />Auto captions</UiButton>
      </div>
    </template>
  </UiSection>
</template>
