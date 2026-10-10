<script setup lang="ts">
import { Expand, Grid3x3, Pause, Play, Shrink, SkipBack, SkipForward, StepBack, StepForward, Upload, Volume2, VolumeX } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, reactive, ref, shallowRef, toRaw, watch } from 'vue'
import UiIconButton from '@/components/ui/UiIconButton.vue'
import UiMenu from '@/components/ui/UiMenu.vue'
import { drag, registerDropTarget, type DragPayload } from '@/lib/dnd'
import { clamp } from '@/lib/engine/math'
import { buildLayer, layerQuad, resolveScene, type Layer } from '@/lib/engine/scene'
import { MediaPool } from '@/lib/player/media-pool'
import { PreviewRenderer } from '@/lib/player/renderer'
import { sceneContext } from '@/lib/player/scene-context'
import { ASPECT_PRESETS, aspectLabel } from '@/lib/project'
import { importIntoProject, formatTime, pickMediaFiles } from '@/services/media'
import { useEditor } from '@/stores/editor'
import { useSettings } from '@/stores/settings'

const emit = defineEmits<{ editText: [] }>()
const editor = useEditor()
const settings = useSettings()
const stage = ref<HTMLElement | null>(null)
const viewport = ref<HTMLElement | null>(null)
const canvas = ref<HTMLCanvasElement | null>(null)
const view = reactive({ width: 0, height: 0 })
const muted = ref(false)
const grid = ref(false)
const fullscreen = ref(false)
const guides = reactive<{ x: number | null; y: number | null }>({ x: null, y: null })
const staticLayers = shallowRef<Layer[]>([])

const pool = new MediaPool()
const renderer = new PreviewRenderer()
let frameRequest = 0

const project = computed(() => editor.project)
const empty = computed(() => !project.value || project.value.tracks.every(track => track.items.length === 0))
const projectScale = computed(() => project.value ? view.width / project.value.width : 1)
const aspectItems = ASPECT_PRESETS.map(preset => ({ id: preset.id, label: `${preset.label}  ·  ${preset.hint}` }))

// ---------------------------------------------------------------- layout

function fit() {
  if (!viewport.value || !project.value) return
  const padding = fullscreen.value ? 0 : 24
  const availableW = Math.max(40, viewport.value.clientWidth - padding * 2)
  const availableH = Math.max(40, viewport.value.clientHeight - padding * 2)
  const ratio = project.value.width / project.value.height
  let width = availableW
  let height = width / ratio
  if (height > availableH) {
    height = availableH
    width = height * ratio
  }
  view.width = Math.floor(width)
  view.height = Math.floor(height)
}

let observer: ResizeObserver | null = null

// ---------------------------------------------------------------- rendering

function requestRender() {
  if (!frameRequest) frameRequest = requestAnimationFrame(render)
}

function render() {
  frameRequest = 0
  const target = canvas.value
  const current = project.value ? toRaw(project.value) : null
  if (!target || !current || !view.width) return
  const ratio = Math.min(window.devicePixelRatio || 1, 2)
  const width = Math.max(1, Math.min(current.width, Math.round(view.width * ratio)))
  const height = Math.max(1, Math.round(width * current.height / current.width))
  if (target.width !== width || target.height !== height) {
    target.width = width
    target.height = height
  }
  const ctx = target.getContext('2d')
  if (!ctx) return
  const assets = editor.assetsById
  const context = sceneContext(assets, current.width)
  const layers = resolveScene(current, editor.currentTime, context)
  const visibleVideos = new Set(layers.filter(layer => layer.item.kind === 'video').map(layer => layer.item.id))
  pool.master = muted.value ? 0 : 1
  pool.sync(current, assets, editor.currentTime, editor.playing, visibleVideos)
  renderer.draw(ctx, current, layers, pool, assets, width / current.width, editor.currentTime)
  updateStaticLayers()
}

function updateStaticLayers() {
  const current = project.value ? toRaw(project.value) : null
  if (!current) return
  const ids = new Set(editor.selectedIds)
  const context = sceneContext(editor.assetsById, current.width)
  const result: Layer[] = []
  current.tracks.forEach((track, trackIndex) => {
    if (track.hidden || track.kind === 'audio') return
    for (const item of track.items) {
      if (item.hidden || !ids.has(item.id) || editor.currentTime < item.start || editor.currentTime >= item.start + item.duration) continue
      const layer = buildLayer(current, item, trackIndex, editor.currentTime, context, { staticOnly: true })
      if (layer) result.push(layer)
    }
  })
  staticLayers.value = result
}

watch(() => [editor.currentTime, editor.playing, editor.revision, editor.selectedIds, view.width, view.height, muted.value], requestRender)
watch(() => editor.playing, playing => { if (playing) pool.unlockAudio() })
watch(() => project.value && [project.value.width, project.value.height], fit)

const handles = computed(() => staticLayers.value.map(layer => ({
  id: layer.item.id,
  primary: layer.item.id === editor.primaryId,
  locked: Boolean(layer.item.locked || editor.find(layer.item.id)?.track.locked),
  points: layerQuad(layer).map(point => ({ x: point.x * projectScale.value, y: point.y * projectScale.value })),
  center: { x: layer.cx * projectScale.value, y: layer.cy * projectScale.value },
  rotation: layer.rotation
})))

function polygonPath(points: Array<{ x: number; y: number }>) {
  return points.map((point, index) => `${index ? 'L' : 'M'}${point.x},${point.y}`).join(' ') + 'Z'
}

// ---------------------------------------------------------------- hit testing & transforms

function toProject(event: PointerEvent) {
  const rect = canvas.value!.getBoundingClientRect()
  return { x: (event.clientX - rect.left) / projectScale.value, y: (event.clientY - rect.top) / projectScale.value }
}

function inside(point: { x: number; y: number }, quad: Array<{ x: number; y: number }>) {
  let sign = 0
  for (let index = 0; index < quad.length; index++) {
    const a = quad[index]
    const b = quad[(index + 1) % quad.length]
    const cross = (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
    if (Math.abs(cross) < 1e-9) continue
    if (sign === 0) sign = Math.sign(cross)
    else if (Math.sign(cross) !== sign) return false
  }
  return true
}

function hitTest(point: { x: number; y: number }) {
  const current = toRaw(project.value!)
  const context = sceneContext(editor.assetsById, current.width)
  const layers = resolveScene(current, editor.currentTime, context)
  const hits = layers.filter(layer => inside(point, layerQuad(layer))).reverse()
  const selectedHit = hits.find(layer => layer.item.id === editor.primaryId)
  return (selectedHit ?? hits[0])?.item.id ?? null
}

function startGesture(event: PointerEvent, mode: 'move' | 'scale' | 'rotate') {
  const current = project.value
  if (!current || !canvas.value) return
  const targets = staticLayers.value.filter(layer => !layer.item.locked && !editor.find(layer.item.id)?.track.locked)
  if (!targets.length) return
  event.preventDefault()
  event.stopPropagation()
  const start = toProject(event)
  const startStates = new Map(targets.map(layer => [layer.item.id, { x: layer.cx, y: layer.cy, scale: layer.scale, rotation: layer.rotation }]))
  const center = targets.length === 1 ? { x: targets[0].cx, y: targets[0].cy } : { x: targets.reduce((sum, layer) => sum + layer.cx, 0) / targets.length, y: targets.reduce((sum, layer) => sum + layer.cy, 0) / targets.length }
  const startDistance = Math.max(1, Math.hypot(start.x - center.x, start.y - center.y))
  const startAngle = Math.atan2(start.y - center.y, start.x - center.x)
  let changed = false
  const snapPx = 6 / projectScale.value

  const move = (moveEvent: PointerEvent) => {
    const point = toProject(moveEvent)
    if (!changed) {
      if (Math.hypot(point.x - start.x, point.y - start.y) * projectScale.value < 2) return
      editor.beginChange()
      changed = true
    }
    if (mode === 'move') {
      let dx = point.x - start.x
      let dy = point.y - start.y
      guides.x = null
      guides.y = null
      if (targets.length === 1 && editor.snapping && !moveEvent.altKey) {
        const layer = targets[0]
        const halfW = (layer.boxW * layer.scale) / 2
        const halfH = (layer.boxH * layer.scale) / 2
        const cx = startStates.get(layer.item.id)!.x + dx
        const cy = startStates.get(layer.item.id)!.y + dy
        for (const [value, target] of [[cx, current.width / 2], [cx - halfW, 0], [cx + halfW, current.width]] as const) {
          if (Math.abs(value - target) < snapPx) { dx += target - value; guides.x = target; break }
        }
        for (const [value, target] of [[cy, current.height / 2], [cy - halfH, 0], [cy + halfH, current.height]] as const) {
          if (Math.abs(value - target) < snapPx) { dy += target - value; guides.y = target; break }
        }
      }
      editor.setTransform(targets.map(layer => {
        const origin = startStates.get(layer.item.id)!
        return { id: layer.item.id, transform: { x: ((origin.x + dx) / current.width) * 100, y: ((origin.y + dy) / current.height) * 100 } }
      }))
    } else if (mode === 'scale') {
      const factor = Math.hypot(point.x - center.x, point.y - center.y) / startDistance
      editor.setTransform(targets.map(layer => ({ id: layer.item.id, transform: { scale: clamp(Math.round(startStates.get(layer.item.id)!.scale * factor * 1000) / 1000, 0.05, 10) } })))
    } else {
      const angle = Math.atan2(point.y - center.y, point.x - center.x)
      editor.setTransform(targets.map(layer => {
        let rotation = startStates.get(layer.item.id)!.rotation + ((angle - startAngle) * 180) / Math.PI
        if (moveEvent.shiftKey) rotation = Math.round(rotation / 15) * 15
        else if (Math.abs(rotation % 90) < 2 || Math.abs(rotation % 90) > 88) rotation = Math.round(rotation / 90) * 90
        return { id: layer.item.id, transform: { rotation: Math.round(rotation * 10) / 10 } }
      }))
    }
  }
  const finish = () => {
    guides.x = null
    guides.y = null
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', finish)
    window.removeEventListener('keydown', cancel, true)
  }
  const cancel = (keyEvent: KeyboardEvent) => {
    if (keyEvent.key !== 'Escape') return
    keyEvent.preventDefault()
    keyEvent.stopPropagation()
    if (changed) editor.undo()
    finish()
  }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', finish)
  window.addEventListener('keydown', cancel, true)
}

function onCanvasPointer(event: PointerEvent) {
  if (event.button !== 0 || !project.value) return
  const id = hitTest(toProject(event))
  if (!id) {
    editor.select(null)
    return
  }
  if (event.shiftKey || event.ctrlKey || event.metaKey) {
    editor.select(id, true)
    return
  }
  if (!editor.selectedIds.includes(id)) editor.select(id)
  updateStaticLayers()
  startGesture(event, 'move')
}

function onDoubleClick(event: MouseEvent) {
  const id = hitTest(toProject(event as PointerEvent))
  if (id && editor.find(id)?.item.kind === 'text') {
    editor.select(id)
    emit('editText')
  }
}

// ---------------------------------------------------------------- controls

function setAspect(id: string) {
  const preset = ASPECT_PRESETS.find(candidate => candidate.id === id)
  if (preset) editor.setCanvas(preset.width, preset.height)
}

async function toggleFullscreen() {
  if (document.fullscreenElement) await document.exitFullscreen()
  else await stage.value?.requestFullscreen()
}

function onFullscreenChange() {
  fullscreen.value = document.fullscreenElement === stage.value
  requestAnimationFrame(fit)
}

async function importMedia() {
  const assets = await importIntoProject(await pickMediaFiles())
  if (assets.length) editor.addAssetsSequence(assets, { start: 0 })
}

function handleDrop(payload: DragPayload) {
  if (payload.type === 'asset') {
    editor.addAsset(payload.asset, { start: editor.currentTime })
    return true
  }
  if (payload.type === 'item') {
    editor.addItem(payload.defaults, { start: editor.currentTime })
    return true
  }
  if (payload.type === 'files') {
    void importIntoProject(payload.paths).then(assets => assets.length && editor.addAssetsSequence(assets, { start: editor.currentTime }))
    return true
  }
  return false
}

let unregisterDrop: (() => void) | null = null

onMounted(() => {
  observer = new ResizeObserver(fit)
  if (viewport.value) observer.observe(viewport.value)
  fit()
  pool.onFrameReady = requestRender
  document.fonts?.addEventListener('loadingdone', onFontsLoaded)
  document.addEventListener('fullscreenchange', onFullscreenChange)
  unregisterDrop = registerDropTarget(() => viewport.value, handleDrop)
  editor.thumbnailProvider = async () => {
    if (!canvas.value || !project.value || empty.value) return null
    const thumb = document.createElement('canvas')
    const width = 360
    thumb.width = width
    thumb.height = Math.round(width * project.value.height / project.value.width)
    thumb.getContext('2d')!.drawImage(canvas.value, 0, 0, thumb.width, thumb.height)
    return thumb.toDataURL('image/jpeg', 0.82)
  }
})

function onFontsLoaded() {
  renderer.text.invalidate()
  requestRender()
}

onBeforeUnmount(() => {
  cancelAnimationFrame(frameRequest)
  observer?.disconnect()
  document.fonts?.removeEventListener('loadingdone', onFontsLoaded)
  document.removeEventListener('fullscreenchange', onFullscreenChange)
  unregisterDrop?.()
  editor.thumbnailProvider = null
  pool.destroy()
  renderer.destroy()
})

defineExpose({ requestRender })
</script>

<template>
  <section ref="stage" class="flex min-w-0 flex-1 flex-col bg-app">
    <div ref="viewport" class="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden" :class="drag.active && 'ring-1 ring-inset ring-accent/40'" @pointerdown.self="editor.select(null)">
      <div v-if="project" class="relative shadow-[0_0_0_1px_rgba(255,255,255,0.06)]" :style="{ width: `${view.width}px`, height: `${view.height}px` }">
        <canvas ref="canvas" class="block size-full" @pointerdown="onCanvasPointer" @dblclick="onDoubleClick" />
        <div v-if="grid" class="pointer-events-none absolute inset-0 [background-image:linear-gradient(rgba(255,255,255,.18)_1px,transparent_1px),linear-gradient(90deg,rgba(255,255,255,.18)_1px,transparent_1px)] [background-size:33.333%_33.333%]" />
        <div v-if="settings.value.safeArea" class="pointer-events-none absolute inset-x-[6%] bottom-[18%] top-[10%] rounded border border-dashed border-white/35" />
        <svg class="pointer-events-none absolute inset-0 size-full overflow-visible">
          <line v-if="guides.x !== null" :x1="guides.x * projectScale" :x2="guides.x * projectScale" y1="0" :y2="view.height" stroke="#f472b6" stroke-width="1" />
          <line v-if="guides.y !== null" :y1="guides.y * projectScale" :y2="guides.y * projectScale" x1="0" :x2="view.width" stroke="#f472b6" stroke-width="1" />
          <g v-for="handle in handles" :key="handle.id">
            <path :d="polygonPath(handle.points)" fill="none" :stroke="handle.primary ? '#4cc9f0' : 'rgba(76,201,240,.55)'" stroke-width="1.5" />
          </g>
        </svg>
        <template v-for="handle in handles" :key="`h-${handle.id}`">
          <template v-if="handle.primary && !handle.locked && handles.length === 1">
            <button
              v-for="(point, index) in handle.points" :key="index"
              class="absolute size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-sm border border-accent bg-white"
              :class="index % 2 === 0 ? 'cursor-nwse-resize' : 'cursor-nesw-resize'"
              :style="{ left: `${point.x}px`, top: `${point.y}px` }"
              aria-label="Scale"
              @pointerdown="startGesture($event, 'scale')"
            />
            <button
              class="absolute grid size-5 -translate-x-1/2 -translate-y-1/2 cursor-grab place-items-center rounded-full border border-accent bg-panel text-accent"
              :style="{ left: `${(handle.points[0].x + handle.points[1].x) / 2 - Math.sin(handle.rotation * Math.PI / 180) * -22}px`, top: `${(handle.points[0].y + handle.points[1].y) / 2 - Math.cos(handle.rotation * Math.PI / 180) * 22}px` }"
              aria-label="Rotate"
              @pointerdown="startGesture($event, 'rotate')"
            >
              <svg viewBox="0 0 16 16" class="size-3" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M13 8a5 5 0 1 1-1.6-3.7M13 3v3h-3" /></svg>
            </button>
          </template>
        </template>
      </div>
      <div v-if="empty" class="absolute flex flex-col items-center gap-3 text-center">
        <p class="text-sm font-medium text-fg">Start by adding media</p>
        <p class="max-w-64 text-xs text-muted">Import videos, photos or music, or drag files from your computer anywhere into the editor.</p>
        <button class="mt-1 flex h-9 items-center gap-2 rounded-md bg-accent px-4 text-xs font-semibold text-accent-ink hover:bg-[#6fd5f4]" @click="importMedia"><Upload class="size-4" />Import media</button>
      </div>
    </div>

    <footer class="flex h-11 shrink-0 items-center gap-1 border-t border-line px-3">
      <span class="w-36 text-xs tabular-nums"><span class="text-fg">{{ formatTime(editor.currentTime, editor.fps) }}</span> <span class="text-faint">/ {{ formatTime(editor.playbackEnd, editor.fps) }}</span></span>
      <div class="mx-auto flex items-center gap-0.5">
        <UiIconButton label="Go to start" shortcut="Home" size="sm" @click="editor.seek(0)"><SkipBack class="size-3.5" /></UiIconButton>
        <UiIconButton label="Previous frame" shortcut="←" size="sm" @click="editor.stepFrames(-1)"><StepBack class="size-3.5" /></UiIconButton>
        <UiIconButton :label="editor.playing ? 'Pause' : 'Play'" shortcut="Space" variant="secondary" @click="pool.unlockAudio(); editor.togglePlay()">
          <Pause v-if="editor.playing" class="size-4 fill-current" /><Play v-else class="size-4 fill-current" />
        </UiIconButton>
        <UiIconButton label="Next frame" shortcut="→" size="sm" @click="editor.stepFrames(1)"><StepForward class="size-3.5" /></UiIconButton>
        <UiIconButton label="Go to end" shortcut="End" size="sm" @click="editor.seek(editor.playbackEnd)"><SkipForward class="size-3.5" /></UiIconButton>
      </div>
      <div class="flex w-36 items-center justify-end gap-0.5">
        <UiMenu v-if="project" :items="aspectItems" side="top" @select="setAspect">
          <button class="h-7 rounded-md px-2 text-xs text-muted hover:bg-hover hover:text-fg" aria-label="Canvas ratio">{{ aspectLabel(project.width, project.height) }}</button>
        </UiMenu>
        <UiIconButton label="Grid" size="sm" :active="grid" @click="grid = !grid"><Grid3x3 class="size-3.5" /></UiIconButton>
        <UiIconButton :label="muted ? 'Unmute preview' : 'Mute preview'" size="sm" @click="muted = !muted"><VolumeX v-if="muted" class="size-3.5" /><Volume2 v-else class="size-3.5" /></UiIconButton>
        <UiIconButton :label="fullscreen ? 'Exit full screen' : 'Full screen'" size="sm" @click="toggleFullscreen"><Shrink v-if="fullscreen" class="size-3.5" /><Expand v-else class="size-3.5" /></UiIconButton>
      </div>
    </footer>
  </section>
</template>
