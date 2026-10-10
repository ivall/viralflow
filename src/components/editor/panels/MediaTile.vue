<script setup lang="ts">
import { AudioLines, CircleAlert, Plus } from '@lucide/vue'
import { computed, ref, watchEffect } from 'vue'
import type { MediaAsset } from '@/lib/types'
import { fileUrl, type Filmstrip } from '@/services/ipc'
import { filmstripFor, formatDuration, proxyProgress } from '@/services/media'

const props = defineProps<{ asset: MediaAsset; offline?: boolean; used?: boolean }>()
const emit = defineEmits<{ add: [] }>()
const strip = ref<Filmstrip | null>(null)

watchEffect(() => {
  strip.value = null
  if (props.asset.kind === 'video' && !props.offline) void filmstripFor(props.asset).then(result => { strip.value = result })
})

const preparing = computed(() => proxyProgress.get(props.asset.path))
const thumbnail = computed(() => {
  if (props.asset.kind === 'image') return { backgroundImage: `url("${fileUrl(props.asset.previewPath ?? props.asset.path)}")`, backgroundSize: 'cover', backgroundPosition: 'center' }
  if (props.asset.kind === 'video' && strip.value) {
    const middle = Math.floor(strip.value.count / 3)
    return {
      backgroundImage: `url("${fileUrl(strip.value.path)}")`,
      backgroundSize: `${strip.value.count * 100}% 100%`,
      backgroundPosition: `${strip.value.count > 1 ? (middle / (strip.value.count - 1)) * 100 : 0}% 0`
    }
  }
  return {}
})
</script>

<template>
  <div class="group relative min-w-0">
    <div class="relative aspect-video overflow-hidden rounded-md border bg-app bg-no-repeat" :class="offline ? 'border-danger/60' : 'border-line group-hover:border-line-strong'" :style="thumbnail">
      <div v-if="asset.kind === 'audio'" class="flex size-full items-center justify-center bg-[color-mix(in_srgb,var(--color-clip-audio)_18%,transparent)]"><AudioLines class="size-6 text-clip-audio" /></div>
      <span v-if="asset.duration && asset.kind !== 'image'" class="absolute bottom-1 right-1 rounded bg-black/70 px-1 text-[10px] tabular-nums text-white">{{ formatDuration(asset.duration) }}</span>
      <span v-if="used" class="absolute left-1 top-1 size-1.5 rounded-full bg-accent" title="Used in the timeline" />
      <div v-if="offline" class="absolute inset-0 flex items-center justify-center gap-1 bg-black/70 text-[10px] font-medium text-danger"><CircleAlert class="size-3.5" />Missing</div>
      <div v-else-if="preparing !== undefined" class="absolute inset-x-0 bottom-0 h-1 bg-black/60"><div class="h-full bg-accent transition-[width]" :style="{ width: `${Math.round(preparing * 100)}%` }" /></div>
      <button v-if="!offline" class="absolute right-1 top-1 grid size-6 place-items-center rounded-md bg-accent text-accent-ink opacity-0 shadow transition-opacity group-hover:opacity-100" aria-label="Add to timeline" title="Add at playhead" @pointerdown.stop @click.stop="emit('add')"><Plus class="size-4" /></button>
    </div>
    <p class="mt-1 truncate text-[11px] text-muted" :title="asset.path">{{ asset.name }}</p>
  </div>
</template>
