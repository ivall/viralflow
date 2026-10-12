<script setup lang="ts">
import { Blend, Captions, Clapperboard, Smile, Sparkles, Type } from '@lucide/vue'
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
  { id: 'text', label: 'Text', icon: Type },
  { id: 'stickers', label: 'Stickers', icon: Smile },
  { id: 'effects', label: 'Effects', icon: Sparkles },
  { id: 'transitions', label: 'Transitions', icon: Blend },
  { id: 'captions', label: 'Captions', icon: Captions }
]
</script>

<template>
  <aside class="surface flex w-[400px] shrink-0 flex-col overflow-hidden">
    <nav class="grid shrink-0 grid-cols-6 gap-0.5 border-b border-line p-1.5" role="tablist" aria-label="Assets">
      <button
        v-for="tab in tabs" :key="tab.id" role="tab" :aria-selected="panel === tab.id" :title="tab.label"
        class="flex h-14 flex-col items-center justify-center gap-1 rounded-lg text-[11px] font-medium transition-colors"
        :class="panel === tab.id ? 'bg-raised text-fg' : 'text-muted hover:bg-hover/50 hover:text-fg'"
        @click="panel = tab.id"
      >
        <component :is="tab.icon" class="size-[18px]" :class="panel === tab.id && 'text-accent'" />
        <span class="max-w-full truncate px-0.5">{{ tab.label }}</span>
      </button>
    </nav>
    <div class="min-h-0 flex-1">
      <KeepAlive>
        <MediaPanel v-if="panel === 'media'" />
        <TextPanel v-else-if="panel === 'text'" @edit="emit('editText')" />
        <StickersPanel v-else-if="panel === 'stickers'" />
        <EffectsPanel v-else-if="panel === 'effects'" />
        <TransitionsPanel v-else-if="panel === 'transitions'" />
        <CaptionsPanel v-else-if="panel === 'captions'" :source-id="captionSource" />
      </KeepAlive>
    </div>
  </aside>
</template>
