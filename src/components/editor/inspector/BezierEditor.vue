<script setup lang="ts">
import { computed, ref } from 'vue'
import { clamp } from '@/lib/engine/math'
import type { Bezier } from '@/lib/types'

/** Drag the two handles of a cubic-bezier curve; y may overshoot 0..1. */
const props = defineProps<{ modelValue: Bezier }>()
const emit = defineEmits<{ start: []; 'update:modelValue': [value: Bezier] }>()

const WIDTH = 200
const HEIGHT = 150
const PAD = 12
const Y_MIN = -0.6
const Y_MAX = 1.6

const svg = ref<SVGSVGElement | null>(null)
const dragging = ref<0 | 1 | null>(null)

const toX = (x: number) => PAD + x * (WIDTH - 2 * PAD)
const toY = (y: number) => PAD + (Y_MAX - y) / (Y_MAX - Y_MIN) * (HEIGHT - 2 * PAD)

const points = computed(() => {
  const [x1, y1, x2, y2] = props.modelValue
  return { start: [toX(0), toY(0)], end: [toX(1), toY(1)], first: [toX(x1), toY(y1)], second: [toX(x2), toY(y2)] }
})
const path = computed(() => {
  const { start, first, second, end } = points.value
  return `M${start} C${first} ${second} ${end}`
})
const label = computed(() => props.modelValue.map(value => Number(value.toFixed(2))).join(', '))

function begin(handle: 0 | 1, event: PointerEvent) {
  dragging.value = handle
  ;(event.currentTarget as Element).setPointerCapture(event.pointerId)
  emit('start')
}

function move(event: PointerEvent) {
  if (dragging.value === null || !svg.value) return
  const box = svg.value.getBoundingClientRect()
  const px = (event.clientX - box.left) / box.width * WIDTH
  const py = (event.clientY - box.top) / box.height * HEIGHT
  const x = Math.round(clamp((px - PAD) / (WIDTH - 2 * PAD), 0, 1) * 100) / 100
  const y = Math.round(clamp(Y_MAX - (py - PAD) / (HEIGHT - 2 * PAD) * (Y_MAX - Y_MIN), Y_MIN, Y_MAX) * 100) / 100
  const next = [...props.modelValue] as Bezier
  next[dragging.value * 2] = x
  next[dragging.value * 2 + 1] = y
  emit('update:modelValue', next)
}
</script>

<template>
  <div class="space-y-1">
    <svg ref="svg" :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" class="w-full touch-none select-none rounded-md bg-app" role="img" :aria-label="`Easing curve cubic-bezier(${label})`" @pointermove="move" @pointerup="dragging = null" @pointercancel="dragging = null">
      <line :x1="toX(0)" :x2="toX(1)" :y1="toY(0)" :y2="toY(0)" class="stroke-line" stroke-dasharray="3 3" />
      <line :x1="toX(0)" :x2="toX(1)" :y1="toY(1)" :y2="toY(1)" class="stroke-line" stroke-dasharray="3 3" />
      <line :x1="points.start[0]" :y1="points.start[1]" :x2="points.first[0]" :y2="points.first[1]" class="stroke-faint" />
      <line :x1="points.end[0]" :y1="points.end[1]" :x2="points.second[0]" :y2="points.second[1]" class="stroke-faint" />
      <path :d="path" fill="none" class="stroke-accent" stroke-width="2" />
      <circle v-for="(point, index) in [points.first, points.second]" :key="index" :cx="point[0]" :cy="point[1]" r="6" class="cursor-grab fill-accent stroke-app active:cursor-grabbing" stroke-width="2" @pointerdown="event => begin(index as 0 | 1, event)" />
    </svg>
    <p class="text-center font-mono text-2xs text-faint">cubic-bezier({{ label }})</p>
  </div>
</template>
