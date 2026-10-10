<script setup lang="ts">
import { computed, ref, watchEffect } from 'vue'
import type { MediaAsset } from '@/lib/types'
import { fileUrl, type Filmstrip } from '@/services/ipc'
import { filmstripFor } from '@/services/media'

/** Thumbnails of a video clip; only tiles inside [from, to] (clip-local px) are rendered. */
const props = defineProps<{ asset: MediaAsset; sourceIn: number; speed: number; zoom: number; height: number; from: number; to: number }>()
const strip = ref<Filmstrip | null>(null)

watchEffect(() => {
  const asset = props.asset
  strip.value = null
  void filmstripFor(asset).then(result => { if (asset === props.asset) strip.value = result })
})

const tiles = computed(() => {
  const data = strip.value
  if (!data) return []
  const tileWidth = (data.tileWidth / data.tileHeight) * props.height
  const first = Math.max(0, Math.floor(props.from / tileWidth))
  const last = Math.ceil(props.to / tileWidth)
  const url = fileUrl(data.path)
  const result: Array<{ key: number; left: number; style: Record<string, string> }> = []
  for (let index = first; index <= last && result.length < 200; index++) {
    const x = index * tileWidth
    const sourceTime = props.sourceIn + ((x + tileWidth / 2) / props.zoom) * props.speed
    const tile = Math.min(data.count - 1, Math.max(0, Math.floor(sourceTime / data.interval)))
    result.push({
      key: index,
      left: x,
      style: {
        width: `${tileWidth}px`,
        backgroundImage: `url("${url}")`,
        backgroundSize: `${data.count * tileWidth}px ${props.height}px`,
        backgroundPosition: `${-tile * tileWidth}px 0`
      }
    })
  }
  return result
})
</script>

<template>
  <div class="pointer-events-none absolute inset-0 overflow-hidden">
    <div v-for="tile in tiles" :key="tile.key" class="absolute inset-y-0 bg-no-repeat" :style="{ left: `${tile.left}px`, ...tile.style }" />
  </div>
</template>
