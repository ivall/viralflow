<script setup lang="ts">
import { AudioLines, Film, Image as ImageIcon, Shapes, Type } from '@lucide/vue'
import { computed, nextTick, ref, watch } from 'vue'
import { TRACK_COLORS } from '@/lib/project'
import { useEditor } from '@/stores/editor'
import AdjustSection from './inspector/AdjustSection.vue'
import AnimationSection from './inspector/AnimationSection.vue'
import AudioSection from './inspector/AudioSection.vue'
import FrameSection from './inspector/FrameSection.vue'
import ProjectSection from './inspector/ProjectSection.vue'
import ShapeSection from './inspector/ShapeSection.vue'
import SpeedSection from './inspector/SpeedSection.vue'
import TextSection from './inspector/TextSection.vue'
import TransformSection from './inspector/TransformSection.vue'

const emit = defineEmits<{ captions: [itemId: string] }>()
const editor = useEditor()
const item = computed(() => editor.selectedItem)
const tab = ref('main')
const textSection = ref<InstanceType<typeof TextSection> | null>(null)

const tabs = computed(() => {
  switch (item.value?.kind) {
    case 'video': return [{ id: 'main', label: 'Video' }, { id: 'animation', label: 'Animation' }, { id: 'adjust', label: 'Adjust' }, { id: 'audio', label: 'Audio' }, { id: 'speed', label: 'Speed' }]
    case 'image': return [{ id: 'main', label: 'Image' }, { id: 'animation', label: 'Animation' }, { id: 'adjust', label: 'Adjust' }]
    case 'text': return [{ id: 'main', label: 'Text' }, { id: 'animation', label: 'Animation' }, { id: 'transform', label: 'Transform' }]
    case 'shape': return [{ id: 'main', label: 'Shape' }, { id: 'animation', label: 'Animation' }]
    case 'audio': return [{ id: 'main', label: 'Audio' }, { id: 'speed', label: 'Speed' }]
    default: return []
  }
})

const icon = computed(() => item.value ? { video: Film, image: ImageIcon, audio: AudioLines, text: Type, shape: Shapes }[item.value.kind] : null)

watch(() => item.value?.kind, () => {
  if (!tabs.value.some(candidate => candidate.id === tab.value)) tab.value = 'main'
})

function focusText() {
  tab.value = 'main'
  void nextTick(() => textSection.value?.focus())
}

defineExpose({ focusText })
</script>

<template>
  <aside class="flex w-[300px] shrink-0 flex-col border-l border-line bg-panel">
    <template v-if="item">
      <header class="flex h-11 shrink-0 items-center gap-2 border-b border-line px-4">
        <span class="grid size-6 place-items-center rounded" :style="{ background: `color-mix(in srgb, ${TRACK_COLORS[item.kind]} 25%, transparent)`, color: TRACK_COLORS[item.kind] }">
          <component :is="icon" class="size-3.5" />
        </span>
        <span class="min-w-0 flex-1 truncate text-xs font-medium">{{ editor.selectedIds.length > 1 ? `${editor.selectedIds.length} clips selected` : item.name }}</span>
      </header>
      <nav class="flex h-9 shrink-0 gap-3 border-b border-line px-4" role="tablist">
        <button
          v-for="entry in tabs" :key="entry.id" role="tab" :aria-selected="tab === entry.id"
          class="relative text-xs transition-colors"
          :class="tab === entry.id ? 'text-fg' : 'text-muted hover:text-fg'"
          @click="tab = entry.id"
        >
          {{ entry.label }}<span v-if="tab === entry.id" class="absolute inset-x-0 bottom-0 h-0.5 rounded-full bg-accent" />
        </button>
      </nav>
      <div class="min-h-0 flex-1 overflow-y-auto">
        <template v-if="tab === 'main'">
          <TextSection v-if="item.kind === 'text'" ref="textSection" :item="item" />
          <AudioSection v-else-if="item.kind === 'audio'" :item="item" @captions="emit('captions', item.id)" />
          <template v-else>
            <ShapeSection v-if="item.kind === 'shape'" :item="item" />
            <TransformSection :item="item" />
            <FrameSection v-if="item.kind === 'video' || item.kind === 'image'" :item="item" />
          </template>
        </template>
        <TransformSection v-else-if="tab === 'transform'" :item="item" />
        <AnimationSection v-else-if="tab === 'animation'" :item="item" />
        <AdjustSection v-else-if="tab === 'adjust'" :item="item" />
        <AudioSection v-else-if="tab === 'audio'" :item="item" @captions="emit('captions', item.id)" />
        <SpeedSection v-else-if="tab === 'speed'" :item="item" />
      </div>
    </template>
    <template v-else-if="editor.project">
      <header class="flex h-11 shrink-0 items-center border-b border-line px-4 text-xs font-medium">Project settings</header>
      <div class="min-h-0 flex-1 overflow-y-auto"><ProjectSection /></div>
    </template>
  </aside>
</template>
