<script setup lang="ts">
import { Blend, Captions, Clapperboard, Music, Smile, Sparkles, Type } from '@lucide/vue'
import AudioPanel from './panels/AudioPanel.vue'
import CaptionsPanel from './panels/CaptionsPanel.vue'
import EffectsPanel from './panels/EffectsPanel.vue'
import MediaPanel from './panels/MediaPanel.vue'
import StickersPanel from './panels/StickersPanel.vue'
import TextPanel from './panels/TextPanel.vue'
import TransitionsPanel from './panels/TransitionsPanel.vue'

defineProps<{ captionSource?: string | null }>()
const panel = defineModel<string>({ required: true })
const emit = defineEmits<{ editText: [] }>()

const tabs = [
  { id: 'media', label: 'Media', icon: Clapperboard },
  { id: 'audio', label: 'Audio', icon: Music },
  { id: 'text', label: 'Text', icon: Type },
  { id: 'stickers', label: 'Stickers', icon: Smile },
  { id: 'effects', label: 'Effects', icon: Sparkles },
  { id: 'transitions', label: 'Transitions', icon: Blend },
  { id: 'captions', label: 'Captions', icon: Captions }
]
</script>

<template>
  <aside class="flex w-[340px] shrink-0 flex-col border-r border-line bg-panel">
    <nav class="grid shrink-0 grid-cols-7 border-b border-line px-1 py-1" role="tablist" aria-label="Assets">
      <button
        v-for="tab in tabs" :key="tab.id" role="tab" :aria-selected="panel === tab.id" :title="tab.label"
        class="flex h-12 flex-col items-center justify-center gap-1 rounded-md text-[10px] transition-colors"
        :class="panel === tab.id ? 'bg-hover text-fg' : 'text-muted hover:text-fg'"
        @click="panel = tab.id"
      >
        <component :is="tab.icon" class="size-4" :class="panel === tab.id && 'text-accent'" />
        <span class="max-w-full truncate px-0.5">{{ tab.label }}</span>
      </button>
    </nav>
    <div class="min-h-0 flex-1">
      <KeepAlive>
        <MediaPanel v-if="panel === 'media'" />
        <AudioPanel v-else-if="panel === 'audio'" />
        <TextPanel v-else-if="panel === 'text'" @edit="emit('editText')" />
        <StickersPanel v-else-if="panel === 'stickers'" />
        <EffectsPanel v-else-if="panel === 'effects'" />
        <TransitionsPanel v-else-if="panel === 'transitions'" />
        <CaptionsPanel v-else-if="panel === 'captions'" :source-id="captionSource" />
      </KeepAlive>
    </div>
  </aside>
</template>
