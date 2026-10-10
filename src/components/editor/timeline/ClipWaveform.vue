<script setup lang="ts">
import { onMounted, ref, shallowRef, watch, watchEffect } from 'vue'
import type { MediaAsset } from '@/lib/types'
import { waveformFor } from '@/services/media'

/** Waveform of an audio range; draws only the visible part [from, to] (clip-local px). */
const props = defineProps<{ asset: MediaAsset; sourceIn: number; speed: number; zoom: number; from: number; to: number; color: string; volume: number }>()
const canvas = ref<HTMLCanvasElement | null>(null)
const peaks = shallowRef<Uint8Array | null>(null)

watchEffect(() => {
  const asset = props.asset
  peaks.value = null
  void waveformFor(asset).then(data => { if (asset === props.asset) peaks.value = data })
})

function draw() {
  const element = canvas.value
  const data = peaks.value
  if (!element) return
  const ratio = Math.min(2, window.devicePixelRatio || 1)
  const width = Math.max(1, Math.round(props.to - props.from))
  const height = element.clientHeight || 30
  element.width = Math.min(8192, width * ratio)
  element.height = height * ratio
  element.style.width = `${width}px`
  element.style.left = `${props.from}px`
  const ctx = element.getContext('2d')!
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0)
  ctx.clearRect(0, 0, width, height)
  if (!data?.length) return
  ctx.fillStyle = props.color
  const gain = Math.min(1.6, Math.max(0.15, props.volume))
  const mid = height / 2
  for (let x = 0; x < width; x += 2) {
    const fromTime = props.sourceIn + ((props.from + x) / props.zoom) * props.speed
    const toTime = props.sourceIn + ((props.from + x + 2) / props.zoom) * props.speed
    const fromIndex = Math.max(0, Math.floor(fromTime * 100))
    const toIndex = Math.min(data.length, Math.max(fromIndex + 1, Math.ceil(toTime * 100)))
    let peak = 0
    for (let index = fromIndex; index < toIndex; index++) peak = Math.max(peak, data[index])
    const amplitude = Math.max(0.5, (peak / 255) * gain * mid)
    ctx.fillRect(x, mid - amplitude, 1.4, amplitude * 2)
  }
}

watch(() => [peaks.value, props.from, props.to, props.zoom, props.sourceIn, props.speed, props.volume, props.color], () => requestAnimationFrame(draw))
onMounted(() => requestAnimationFrame(draw))
</script>

<template>
  <canvas ref="canvas" class="pointer-events-none absolute inset-y-0 h-full" />
</template>
