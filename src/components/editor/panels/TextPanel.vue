<script setup lang="ts">
import { Plus } from '@lucide/vue'
import { computed, ref } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import { beginDrag } from '@/lib/dnd'
import { TEXT_PRESET_CATEGORIES, TEXT_PRESETS, textItemDefaults, type TextPreset, type TextPresetCategory } from '@/lib/project'
import { previewWords, textPreviewStyle } from '@/lib/textPreview'
import { useEditor } from '@/stores/editor'

const emit = defineEmits<{ edit: [] }>()
const editor = useEditor()
const category = ref<'All' | TextPresetCategory>('All')
const categories = ['All', ...TEXT_PRESET_CATEGORIES] as const
const presets = computed(() => category.value === 'All' ? TEXT_PRESETS : TEXT_PRESETS.filter(preset => preset.category === category.value))

function add(preset?: TextPreset) {
  editor.addItem(textItemDefaults(preset?.sample, preset))
  emit('edit')
}

function onPointer(event: PointerEvent, preset: TextPreset) {
  const defaults = textItemDefaults(preset.sample, preset)
  beginDrag(event, { type: 'item', defaults, duration: defaults.duration ?? 3 }, preset.label, () => add(preset))
}

/** Long samples are drawn a little smaller so they fit the card. */
function previewSize(preset: TextPreset) {
  const length = preset.sample.length
  return length > 20 ? 13 : length > 12 ? 15 : 18
}
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="shrink-0 space-y-3 p-3">
      <UiButton variant="primary" class="w-full" @click="add()"><Plus class="size-4" />Add text</UiButton>
      <div class="flex flex-wrap gap-1" role="radiogroup" aria-label="Style category">
        <button
          v-for="entry in categories" :key="entry"
          type="button" role="radio" :aria-checked="category === entry"
          class="h-7 rounded-full px-3 text-xs transition-colors"
          :class="category === entry ? 'bg-fg font-medium text-app' : 'bg-raised text-muted hover:text-fg'"
          @click="category = entry"
        >
          {{ entry }}
        </button>
      </div>
    </div>
    <div class="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
      <div class="grid grid-cols-2 gap-2">
        <button
          v-for="preset in presets" :key="preset.id"
          class="group flex flex-col gap-1.5 text-left"
          :title="`${preset.label}: click to add, or drag to the timeline`"
          @pointerdown="onPointer($event, preset)"
        >
          <span class="card flex h-[84px] w-full items-center justify-center overflow-hidden rounded-lg border border-line px-2.5 text-center transition-colors group-hover:border-line-strong">
            <span class="line-clamp-2 max-w-full break-words" :style="textPreviewStyle(preset.style, previewSize(preset))"><span v-for="(word, index) in previewWords(preset.style, preset.sample)" :key="index" :style="word.style">{{ word.text }}</span></span>
          </span>
          <span class="truncate px-0.5 text-2xs text-muted group-hover:text-fg">{{ preset.label }}</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* A muted "footage" backdrop, so white, dark and glowing styles all read. */
.card {
  background:
    radial-gradient(120% 90% at 20% 0%, rgb(255 255 255 / 0.07), transparent 60%),
    linear-gradient(160deg, #2a2d36, #1a1b21 70%);
}
</style>
