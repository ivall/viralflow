<script setup lang="ts">
import {
  ArrowLeftToLine, ArrowRightToLine, AudioLines, Copy, Diamond, Eye, EyeOff, Film, Group, Lock, Magnet, Maximize2, MoreHorizontal, Pencil,
  Plus, Redo2, Scissors, Shapes, SquareSplitHorizontal, Trash2, Type, Undo2, Ungroup, Unlock, Volume2, VolumeX, ZoomIn, ZoomOut, Image as ImageIcon, Blend, Sparkles, X, GripVertical
} from '@lucide/vue'
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import UiContextMenu from '@/components/ui/UiContextMenu.vue'
import UiIconButton from '@/components/ui/UiIconButton.vue'
import UiMenu from '@/components/ui/UiMenu.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import type { MenuItem } from '@/components/ui/menu'
import { drag, dragDuration, dragKind, registerDropTarget, type DragPayload } from '@/lib/dnd'
import { clamp } from '@/lib/engine/math'
import { sortedKeyframes } from '@/lib/engine/keyframes'
import { MAX_TRANSITION, MIN_TRANSITION, TRANSITIONS, trackSeams, type Seam } from '@/lib/engine/transitions'
import { EFFECTS, effectWindow, MIN_EFFECT } from '@/lib/engine/effects'
import { trackKindFor, TRACK_COLORS } from '@/lib/project'
import type { TimelineItem, Track, TrackKind } from '@/lib/types'
import { fileUrl } from '@/services/ipc'
import { formatTime, importIntoProject } from '@/services/media'
import { useEditor } from '@/stores/editor'
import ClipFilmstrip from './ClipFilmstrip.vue'
import ClipWaveform from './ClipWaveform.vue'

const props = defineProps<{ height: number }>()
const emit = defineEmits<{ openPanel: [panel: string]; resize: [height: number] }>()
const editor = useEditor()

const HEADER_W = 200
const PAD = 16
const RULER_H = 30
const GAP = 4
const SNAP_PX = 8

const scroller = ref<HTMLElement | null>(null)
const headerColumn = ref<HTMLElement | null>(null)
const scroll = reactive({ left: 0, top: 0, width: 800, height: 300 })
const snapLine = ref<number | null>(null)
const marquee = ref<{ x0: number; y0: number; x1: number; y1: number } | null>(null)
const editingTrack = ref<string | null>(null)
const trackDraft = ref('')
const dropHint = ref<{ kind: 'track'; trackId: string; start: number; duration: number } | { kind: 'new'; top: number; start: number; duration: number } | { kind: 'seam'; seam: Seam } | { kind: 'clip'; itemId: string } | null>(null)
const resizingSeam = ref<string | null>(null)
const trackDrag = ref<{ id: string; index: number } | null>(null)

const tracks = computed(() => editor.tracks)
const dropRow = computed(() => {
  const hint = dropHint.value
  return hint?.kind === 'track' ? rows.value.find(row => row.track.id === hint.trackId) ?? null : null
})
const zoom = computed(() => editor.zoom)

function rowHeight(track: Track) {
  return track.kind === 'video' ? 72 : track.kind === 'audio' ? 54 : 38
}

const rows = computed(() => {
  let top = 8
  return tracks.value.map(track => {
    const row = { track, top, height: rowHeight(track) }
    top += row.height + GAP
    return row
  })
})
const rowsHeight = computed(() => (rows.value.at(-1) ? rows.value.at(-1)!.top + rows.value.at(-1)!.height : 0) + 48)
const contentWidth = computed(() => PAD * 2 + Math.max(editor.timelineLength * zoom.value, scroll.width))
const viewStart = computed(() => (scroll.left - PAD) / zoom.value - 2)
const viewEnd = computed(() => (scroll.left + scroll.width - PAD) / zoom.value + 2)
const playheadX = computed(() => PAD + editor.currentTime * zoom.value)

function x(time: number) {
  return PAD + time * zoom.value
}

function timeAt(clientX: number) {
  const rect = scroller.value!.getBoundingClientRect()
  return Math.max(0, (clientX - rect.left + scroller.value!.scrollLeft - PAD) / zoom.value)
}

function rowAt(clientY: number) {
  const rect = scroller.value!.getBoundingClientRect()
  const y = clientY - rect.top + scroller.value!.scrollTop - RULER_H
  return { y, row: rows.value.find(row => y >= row.top - GAP / 2 && y < row.top + row.height + GAP / 2) ?? null }
}

function visibleItems(track: Track) {
  return track.items.filter(item => item.start + item.duration >= viewStart.value && item.start <= viewEnd.value)
}

function onScroll() {
  const element = scroller.value
  if (!element) return
  scroll.left = element.scrollLeft
  scroll.top = element.scrollTop
  scroll.width = element.clientWidth
  scroll.height = element.clientHeight
  if (headerColumn.value) headerColumn.value.scrollTop = element.scrollTop
}

// ---------------------------------------------------------------- ruler

const ruler = computed(() => {
  const steps = [1 / 30, 0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600]
  const major = steps.find(step => step * zoom.value >= 90) ?? 600
  const minor = major / (major >= 1 && major % 5 === 0 ? 5 : major >= 1 ? 2 : 5)
  const from = Math.max(0, Math.floor(viewStart.value / minor))
  const to = Math.ceil(viewEnd.value / minor)
  const ticks: Array<{ time: number; major: boolean }> = []
  for (let index = from; index <= to && ticks.length < 600; index++) {
    const time = index * minor
    ticks.push({ time, major: Math.abs(time / major - Math.round(time / major)) < 1e-6 })
  }
  return { ticks, major }
})

function rulerLabel(time: number, major: number) {
  if (major < 1) return formatTime(time, editor.fps)
  return formatTime(time)
}

// ---------------------------------------------------------------- snapping

function snapPoints(exclude: Set<string>) {
  const points = [0, editor.currentTime]
  for (const track of tracks.value) {
    for (const item of track.items) {
      if (exclude.has(item.id)) continue
      points.push(item.start, item.start + item.duration)
    }
  }
  return points
}

function snap(value: number, points: number[], disabled = false) {
  if (!editor.snapping || disabled) return { value, line: null as number | null }
  const threshold = SNAP_PX / zoom.value
  let best = value
  let distance = threshold
  for (const point of points) {
    const delta = Math.abs(point - value)
    if (delta < distance) {
      distance = delta
      best = point
    }
  }
  return { value: best, line: best !== value ? best : null }
}

// ---------------------------------------------------------------- auto scroll

let autoScrollFrame = 0
let lastPointer = { x: 0, y: 0 }
function autoScroll(clientX: number, clientY: number, onScrollStep?: () => void) {
  lastPointer = { x: clientX, y: clientY }
  if (autoScrollFrame) return
  const stepFn = () => {
    autoScrollFrame = 0
    const element = scroller.value
    if (!element) return
    const rect = element.getBoundingClientRect()
    let dx = 0
    let dy = 0
    if (lastPointer.x > rect.right - 40) dx = Math.min(24, (lastPointer.x - (rect.right - 40)) / 2)
    else if (lastPointer.x < rect.left + 40) dx = -Math.min(24, (rect.left + 40 - lastPointer.x) / 2)
    if (lastPointer.y > rect.bottom - 24) dy = 8
    else if (lastPointer.y < rect.top + RULER_H + 12) dy = -8
    if (!dx && !dy) return
    element.scrollLeft += dx
    element.scrollTop += dy
    onScrollStep?.()
    autoScrollFrame = requestAnimationFrame(stepFn)
  }
  autoScrollFrame = requestAnimationFrame(stepFn)
}

function stopAutoScroll() {
  cancelAnimationFrame(autoScrollFrame)
  autoScrollFrame = 0
}

function bindDrag(move: (event: PointerEvent) => void, finish: (cancelled: boolean) => void, cursor = '') {
  let done = false
  const up = () => end(false)
  const key = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') return
    event.preventDefault()
    event.stopPropagation()
    end(true)
  }
  function end(cancelled: boolean) {
    if (done) return
    done = true
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    window.removeEventListener('keydown', key, true)
    document.body.style.cursor = ''
    stopAutoScroll()
    snapLine.value = null
    finish(cancelled)
  }
  if (cursor) document.body.style.cursor = cursor
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
  window.addEventListener('keydown', key, true)
}

// ---------------------------------------------------------------- playhead scrubbing

function startScrub(event: PointerEvent) {
  if (event.button !== 0) return
  event.preventDefault()
  if (editor.playing) editor.stop()
  const points = snapPoints(new Set())
  const apply = (clientX: number, shift: boolean) => {
    const result = snap(timeAt(clientX), points.slice(2).concat(0), shift)
    snapLine.value = result.line
    editor.seek(result.value)
  }
  apply(event.clientX, event.shiftKey)
  bindDrag(move => { autoScroll(move.clientX, move.clientY); apply(move.clientX, move.shiftKey) }, () => {}, 'ew-resize')
}

// ---------------------------------------------------------------- clips

function startClip(event: PointerEvent, item: TimelineItem, track: Track) {
  if (event.button !== 0) return
  event.stopPropagation()
  if (event.shiftKey || event.ctrlKey || event.metaKey) {
    editor.select(item.id, true)
    return
  }
  const wasSelected = editor.selectedIds.includes(item.id)
  if (!wasSelected) editor.select(item.id)
  else editor.primaryId = item.id
  if (item.locked || track.locked) return
  const ids = editor.selectedIds.filter(id => {
    const found = editor.find(id)
    return found && !found.item.locked && !found.track.locked
  })
  const origins = new Map(ids.map(id => [id, editor.find(id)!.item.start]))
  const exclude = new Set(ids)
  const points = snapPoints(exclude)
  const startX = event.clientX
  const startScroll = scroller.value!.scrollLeft
  let moved = false
  let lastClient = { x: event.clientX, y: event.clientY, shift: false }

  const apply = () => {
    const dt = (lastClient.x - startX + scroller.value!.scrollLeft - startScroll) / zoom.value
    const origin = origins.get(item.id)!
    let start = Math.max(0, origin + dt)
    const startSnap = snap(start, points, lastClient.shift)
    const endSnap = snap(start + item.duration, points, lastClient.shift)
    if (startSnap.line !== null && (endSnap.line === null || Math.abs(startSnap.value - start) <= Math.abs(endSnap.value - start - item.duration))) {
      start = startSnap.value
      snapLine.value = startSnap.line
    } else if (endSnap.line !== null) {
      start = endSnap.value - item.duration
      snapLine.value = endSnap.line
    } else snapLine.value = null
    const delta = start - origin
    if (ids.length === 1) {
      const { row } = rowAt(lastClient.y)
      const target = row && row.track.kind === track.kind && !row.track.locked ? row.track.id : undefined
      editor.moveItem(item.id, start, target)
    } else editor.moveItems(ids.map(id => ({ id, start: Math.max(0, origins.get(id)! + delta) })))
  }

  bindDrag(move => {
    lastClient = { x: move.clientX, y: move.clientY, shift: move.shiftKey }
    if (!moved) {
      if (Math.abs(move.clientX - startX) < 3 && Math.abs(move.clientY - event.clientY) < 3) return
      moved = true
      editor.beginChange()
    }
    autoScroll(move.clientX, move.clientY, apply)
    apply()
  }, cancelled => {
    if (cancelled && moved) editor.undo()
    else if (!moved && wasSelected && editor.selectedIds.length > 1 && !item.groupId) editor.select(item.id)
    if (!moved && !editor.playing) seekInto(item)
  }, 'grabbing')
}

function seekInto(item: TimelineItem) {
  if (editor.currentTime < item.start || editor.currentTime >= item.start + item.duration) editor.seek(item.start)
}

function startTrim(event: PointerEvent, item: TimelineItem, edge: 'start' | 'end') {
  if (event.button !== 0) return
  event.stopPropagation()
  event.preventDefault()
  editor.select(item.id)
  const origin = { start: item.start, duration: item.duration, sourceIn: item.sourceIn ?? 0, effectRange: item.effectRange && { ...item.effectRange } }
  const points = snapPoints(new Set([item.id]))
  const startX = event.clientX
  const startScroll = scroller.value!.scrollLeft
  let moved = false
  let last = { x: event.clientX, shift: false }
  const apply = () => {
    const dt = (last.x - startX + scroller.value!.scrollLeft - startScroll) / zoom.value
    const raw = edge === 'start' ? origin.start + dt : origin.start + origin.duration + dt
    const result = snap(raw, points, last.shift)
    snapLine.value = result.line
    editor.trimItem(item.id, edge, result.value, origin)
  }
  bindDrag(move => {
    last = { x: move.clientX, shift: move.shiftKey }
    if (!moved) { moved = true; editor.beginChange() }
    autoScroll(move.clientX, move.clientY, apply)
    apply()
  }, cancelled => { if (cancelled && moved) editor.undo() }, 'ew-resize')
}

function startKeyframe(event: PointerEvent, item: TimelineItem, keyframeId: string, time: number) {
  event.stopPropagation()
  event.preventDefault()
  editor.select(item.id)
  editor.seek(item.start + time)
  const startX = event.clientX
  let moved = false
  bindDrag(move => {
    if (!moved) { moved = true; editor.beginChange() }
    const next = clamp(time + (move.clientX - startX) / zoom.value, 0, item.duration)
    editor.moveKeyframe(item.id, keyframeId, next)
    editor.seek(item.start + next)
  }, cancelled => { if (cancelled && moved) editor.undo() }, 'ew-resize')
}

// ---------------------------------------------------------------- empty area: marquee / seek

function startArea(event: PointerEvent) {
  if (event.button !== 0) return
  const rect = scroller.value!.getBoundingClientRect()
  const toContent = (clientX: number, clientY: number) => ({ x: clientX - rect.left + scroller.value!.scrollLeft, y: clientY - rect.top + scroller.value!.scrollTop - RULER_H })
  const origin = toContent(event.clientX, event.clientY)
  const additive = event.shiftKey || event.ctrlKey || event.metaKey
  const previous = [...editor.selectedIds]
  let moved = false
  bindDrag(move => {
    const point = toContent(move.clientX, move.clientY)
    if (!moved && Math.hypot(point.x - origin.x, point.y - origin.y) < 4) return
    moved = true
    autoScroll(move.clientX, move.clientY)
    marquee.value = { x0: origin.x, y0: origin.y, x1: point.x, y1: point.y }
    const left = Math.min(origin.x, point.x)
    const right = Math.max(origin.x, point.x)
    const top = Math.min(origin.y, point.y)
    const bottom = Math.max(origin.y, point.y)
    const ids = rows.value.flatMap(row => row.top + row.height < top || row.top > bottom ? [] : row.track.items.filter(item => x(item.start) < right && x(item.start + item.duration) > left).map(item => item.id))
    editor.selectMany(additive ? [...previous, ...ids] : ids)
  }, () => {
    marquee.value = null
    if (!moved) {
      if (!additive) editor.select(null)
      if (editor.playing) editor.stop()
      editor.seek(timeAt(event.clientX))
    }
  })
}

// ---------------------------------------------------------------- zoom & wheel

function setZoom(value: number, anchorClientX?: number) {
  const element = scroller.value
  const next = clamp(value, 2, 600)
  if (!element) {
    editor.zoom = next
    return
  }
  const rect = element.getBoundingClientRect()
  const anchorX = anchorClientX !== undefined ? anchorClientX - rect.left : clamp(playheadX.value - element.scrollLeft, 0, element.clientWidth)
  const anchorTime = (anchorX + element.scrollLeft - PAD) / zoom.value
  editor.zoom = next
  void nextTick(() => { element.scrollLeft = Math.max(0, anchorTime * next + PAD - anchorX); onScroll() })
}

function zoomToFit() {
  const end = Math.max(5, editor.playbackEnd)
  setZoom((scroll.width - PAD * 2 - 40) / end)
  void nextTick(() => { if (scroller.value) scroller.value.scrollLeft = 0 })
}

function onWheel(event: WheelEvent) {
  const element = scroller.value
  if (!element) return
  if (event.ctrlKey || event.metaKey) {
    event.preventDefault()
    setZoom(zoom.value * Math.exp(-event.deltaY * 0.0015), event.clientX)
  } else if (event.shiftKey || (Math.abs(event.deltaX) < Math.abs(event.deltaY) && element.scrollHeight <= element.clientHeight + 1)) {
    event.preventDefault()
    element.scrollLeft += event.deltaY || event.deltaX
  }
}

const zoomSlider = computed({
  get: () => Math.log(zoom.value),
  set: (value: number) => setZoom(Math.exp(value))
})

// ---------------------------------------------------------------- playback follow

watch(() => editor.currentTime, time => {
  const element = scroller.value
  if (!element) return
  const px = x(time)
  if (editor.playing) {
    if (px > element.scrollLeft + element.clientWidth - 60 || px < element.scrollLeft) element.scrollLeft = Math.max(0, px - 80)
  }
})

// ---------------------------------------------------------------- transitions (seams)

const seamsByTrack = computed(() => new Map(tracks.value.map(track => [track.id, trackSeams(track, editor.fps).filter(seam => seam.cut >= viewStart.value && seam.cut <= viewEnd.value)])))

function transitionLabel(seam: Seam) {
  return seam.kind === 'none' ? 'Add transition' : `${TRANSITIONS.find(preset => preset.kind === seam.kind)?.label} · ${seam.duration.toFixed(1)}s`
}

function openSeam(seam: Seam) {
  editor.select(seam.to.id)
  emit('openPanel', 'transitions')
}

function seamEditable(seam: Seam, track: Track) {
  return !track.locked && !seam.to.locked
}

/** Dragging either edge stretches the transition symmetrically around the cut. */
function startTransitionResize(event: PointerEvent, seam: Seam, edge: 'start' | 'end') {
  if (event.button !== 0) return
  event.stopPropagation()
  event.preventDefault()
  editor.select(seam.to.id)
  const limit = Math.min(MAX_TRANSITION, seam.from.duration, seam.to.duration)
  let moved = false
  resizingSeam.value = seam.to.id
  bindDrag(move => {
    if (!moved) { moved = true; editor.beginChange() }
    const time = timeAt(move.clientX)
    const half = edge === 'end' ? time - seam.cut : seam.cut - time
    const duration = clamp(Math.round(half * 2 * 20) / 20, MIN_TRANSITION, limit)
    editor.setTransition(seam.to.id, seam.kind, duration, false)
  }, cancelled => {
    resizingSeam.value = null
    if (cancelled && moved) editor.undo()
  }, 'ew-resize')
}

// ---------------------------------------------------------------- effects on clips

function effectLabel(item: TimelineItem) {
  return EFFECTS.find(effect => effect.kind === item.effect)?.label ?? 'Effect'
}

function hasEffect(item: TimelineItem) {
  return Boolean(item.effect && item.effect !== 'none' && item.kind !== 'audio' && item.kind !== 'shape')
}

function acceptsEffect(item: TimelineItem, track: Track) {
  return (item.kind === 'video' || item.kind === 'image' || item.kind === 'text') && !item.locked && !track.locked
}

/** Moves the effect inside its clip, or stretches one of its edges. */
function startEffect(event: PointerEvent, item: TimelineItem, track: Track, mode: 'move' | 'start' | 'end') {
  if (event.button !== 0) return
  event.stopPropagation()
  event.preventDefault()
  editor.select(item.id)
  if (item.locked || track.locked) {
    emit('openPanel', 'effects')
    return
  }
  const origin = effectWindow(item)
  const points = snapPoints(new Set()).filter(point => point >= item.start - 1e-6 && point <= item.start + item.duration + 1e-6)
  const startX = event.clientX
  let moved = false
  bindDrag(move => {
    if (!moved) {
      if (Math.abs(move.clientX - startX) < 3) return
      moved = true
      editor.beginChange()
    }
    const dt = (move.clientX - startX) / zoom.value
    const end = origin.start + origin.duration
    let start = origin.start
    let duration = origin.duration
    if (mode === 'move') {
      start = clamp(origin.start + dt, 0, item.duration - origin.duration)
      const head = snap(item.start + start, points, move.shiftKey)
      const tail = snap(item.start + start + duration, points, move.shiftKey)
      if (head.line !== null) { start = head.value - item.start; snapLine.value = head.line }
      else if (tail.line !== null) { start = tail.value - item.start - duration; snapLine.value = tail.line }
      else snapLine.value = null
      start = clamp(start, 0, item.duration - duration)
    } else if (mode === 'start') {
      const result = snap(item.start + clamp(origin.start + dt, 0, end - MIN_EFFECT), points, move.shiftKey)
      snapLine.value = result.line
      start = clamp(result.value - item.start, 0, end - MIN_EFFECT)
      duration = end - start
    } else {
      const result = snap(item.start + clamp(end + dt, origin.start + MIN_EFFECT, item.duration), points, move.shiftKey)
      snapLine.value = result.line
      duration = clamp(result.value - item.start, origin.start + MIN_EFFECT, item.duration) - origin.start
    }
    editor.setEffectRange(item.id, { start, duration })
  }, cancelled => {
    if (cancelled && moved) editor.undo()
    if (!moved) emit('openPanel', 'effects')
  }, mode === 'move' ? 'grabbing' : 'ew-resize')
}

function removeEffect(item: TimelineItem) {
  editor.setEffect([item.id], 'none')
}

// ---------------------------------------------------------------- reordering tracks

function startTrackDrag(event: PointerEvent, track: Track) {
  if (event.button !== 0 || editingTrack.value === track.id) return
  const target = event.target as HTMLElement
  if (target.closest('input') || target.closest('button:not([data-track-grab])')) return
  const startY = event.clientY
  let moved = false
  let lastY = startY
  const update = () => {
    const column = headerColumn.value
    if (!column) return
    const y = lastY - column.getBoundingClientRect().top + column.scrollTop
    const index = rows.value.findIndex(row => y < row.top + row.height / 2)
    trackDrag.value = { id: track.id, index: index < 0 ? rows.value.length : index }
  }
  bindDrag(move => {
    if (!moved && Math.abs(move.clientY - startY) < 4) return
    moved = true
    lastY = move.clientY
    const rect = scroller.value?.getBoundingClientRect()
    if (rect) autoScroll(rect.left + rect.width / 2, move.clientY, update)
    update()
  }, cancelled => {
    const state = trackDrag.value
    trackDrag.value = null
    if (cancelled || !moved || !state) return
    const from = tracks.value.findIndex(candidate => candidate.id === track.id)
    const to = state.index > from ? state.index - 1 : state.index
    if (from >= 0 && to !== from) editor.moveTrack(track.id, to)
  }, 'grabbing')
}

const trackDropLine = computed(() => {
  const state = trackDrag.value
  if (!state) return null
  const from = tracks.value.findIndex(track => track.id === state.id)
  if (state.index === from || state.index === from + 1) return null
  const row = rows.value[state.index]
  const last = rows.value.at(-1)
  return row ? row.top - GAP / 2 : last ? last.top + last.height + GAP / 2 : 8
})

// ---------------------------------------------------------------- drop from panels and the OS

function computeDrop(payload: DragPayload, clientX: number, clientY: number) {
  if (!scroller.value) return null
  const rect = scroller.value.getBoundingClientRect()
  if (clientX < rect.left || clientX > rect.right || clientY < rect.top || clientY > rect.bottom) return null
  const { y, row } = rowAt(clientY)
  if (payload.type === 'transition') {
    if (!row) return null
    const time = timeAt(clientX)
    const seam = trackSeams(row.track, editor.fps).find(candidate => Math.abs(candidate.cut - time) * zoom.value < 40)
    return seam && seamEditable(seam, row.track) ? { kind: 'seam' as const, seam } : null
  }
  if (payload.type === 'effect') {
    if (!row || y < row.top || y > row.top + row.height) return null
    const time = timeAt(clientX)
    const item = row.track.items.find(candidate => time >= candidate.start && time < candidate.start + candidate.duration && acceptsEffect(candidate, row.track))
    return item ? { kind: 'clip' as const, itemId: item.id } : null
  }
  const kind = payload.type === 'files' ? 'video' : dragKind(payload)
  if (!kind) return null
  const duration = dragDuration(payload)
  const start = Math.max(0, timeAt(clientX) - (payload.type === 'files' ? 0 : 0))
  if (row && row.track.kind === trackKindFor(kind) && !row.track.locked && y >= row.top && y <= row.top + row.height) {
    return { kind: 'track' as const, trackId: row.track.id, start, duration }
  }
  const index = rows.value.findIndex(candidate => y < candidate.top + candidate.height / 2)
  const insertAt = index < 0 ? rows.value.length : index
  const top = insertAt < rows.value.length ? rows.value[insertAt].top - GAP / 2 : (rows.value.at(-1) ? rows.value.at(-1)!.top + rows.value.at(-1)!.height + GAP / 2 : 8)
  return { kind: 'new' as const, top, start, duration, index: insertAt }
}

watch(() => drag.active && drag.payload ? [drag.x, drag.y] : null, () => {
  if (!drag.active || !drag.payload) {
    dropHint.value = null
    return
  }
  const result = computeDrop(drag.payload, drag.x, drag.y)
  dropHint.value = result ? (result.kind === 'new' ? { kind: 'new', top: result.top, start: result.start, duration: result.duration } : result) : null
  if (result) autoScroll(drag.x, drag.y)
})

async function handleDrop(payload: DragPayload, clientX: number, clientY: number) {
  const result = computeDrop(payload, clientX, clientY)
  dropHint.value = null
  if (!result) return false
  if (result.kind === 'seam') {
    if (payload.type === 'transition') {
      editor.setTransition(result.seam.to.id, payload.kind)
      editor.select(result.seam.to.id)
    }
    return true
  }
  if (result.kind === 'clip') {
    if (payload.type === 'effect') {
      editor.setEffect([result.itemId], payload.kind)
      editor.select(result.itemId)
    }
    return true
  }
  const placement = result.kind === 'track' ? { trackId: result.trackId, start: result.start } : { newTrackIndex: result.index, start: result.start }
  if (payload.type === 'asset') editor.addAsset(payload.asset, placement)
  else if (payload.type === 'item') editor.addItem(payload.defaults, placement)
  else if (payload.type === 'files') {
    const assets = await importIntoProject(payload.paths)
    if (!assets.length) return true
    const sameKind = result.kind === 'track' && assets.every(asset => trackKindFor(asset.kind) === editor.tracks.find(track => track.id === result.trackId)?.kind)
    editor.addAssetsSequence(assets, sameKind ? placement : { start: result.start })
  }
  return true
}

// ---------------------------------------------------------------- context menu

const contextItems = computed<MenuItem[]>(() => {
  const count = editor.selectedIds.length
  const item = editor.selectedItem
  const hasMedia = editor.selectedItems.some(candidate => candidate.kind === 'video' || candidate.kind === 'audio')
  return [
    { id: 'split', label: 'Split', icon: Scissors, shortcut: 'S', disabled: !count },
    { id: 'duplicate', label: 'Duplicate', icon: Copy, shortcut: 'Ctrl+D', disabled: !count },
    { id: 'copy', label: 'Copy', shortcut: 'Ctrl+C', disabled: !count },
    { id: 'cut', label: 'Cut', shortcut: 'Ctrl+X', disabled: !count },
    { id: 'paste', label: 'Paste at playhead', shortcut: 'Ctrl+V' },
    { id: 'group', label: 'Group', icon: Group, shortcut: 'Ctrl+G', disabled: count < 2, separatorBefore: true },
    { id: 'ungroup', label: 'Ungroup', icon: Ungroup, shortcut: 'Ctrl+Shift+G', disabled: !editor.selectedItems.some(candidate => candidate.groupId) },
    { id: 'hide', label: item?.hidden ? 'Show clip' : 'Hide clip', icon: item?.hidden ? Eye : EyeOff, disabled: !count, separatorBefore: true },
    { id: 'mute', label: item?.muted ? 'Unmute clip' : 'Mute clip', icon: item?.muted ? Volume2 : VolumeX, disabled: !hasMedia },
    { id: 'lock', label: item?.locked ? 'Unlock clip' : 'Lock clip', icon: item?.locked ? Unlock : Lock, disabled: !count },
    { id: 'ripple', label: 'Delete and close gap', icon: SquareSplitHorizontal, shortcut: 'Shift+Del', disabled: !count, separatorBefore: true },
    { id: 'delete', label: 'Delete', icon: Trash2, shortcut: 'Del', danger: true, disabled: !count }
  ]
})

function onContextMenu(event: MouseEvent) {
  const element = (event.target as HTMLElement).closest<HTMLElement>('[data-item-id]')
  const id = element?.dataset.itemId
  if (id && !editor.selectedIds.includes(id)) editor.select(id)
}

function onContextAction(id: string) {
  switch (id) {
    case 'split': editor.splitAt(); break
    case 'duplicate': editor.duplicateSelected(); break
    case 'copy': editor.copySelected(); break
    case 'cut': editor.cutSelected(); break
    case 'paste': editor.paste(); break
    case 'group': editor.groupSelected(); break
    case 'ungroup': editor.ungroupSelected(); break
    case 'hide': editor.toggleItemFlag('hidden'); break
    case 'mute': editor.toggleItemFlag('muted'); break
    case 'lock': editor.toggleItemFlag('locked'); break
    case 'ripple': editor.deleteSelected(true); break
    case 'delete': editor.deleteSelected(); break
  }
}

// ---------------------------------------------------------------- tracks

const trackIcons = { video: Film, text: Type, audio: AudioLines }
const addTrackItems: MenuItem[] = [
  { id: 'video', label: 'Video track', icon: Film },
  { id: 'text', label: 'Text track', icon: Type },
  { id: 'audio', label: 'Audio track', icon: AudioLines }
]
const trackMenu: MenuItem[] = [
  { id: 'rename', label: 'Rename', icon: Pencil },
  { id: 'up', label: 'Move up' },
  { id: 'down', label: 'Move down' },
  { id: 'delete', label: 'Delete track', icon: Trash2, danger: true, separatorBefore: true }
]

function trackAction(track: Track, action: string) {
  const index = tracks.value.indexOf(track)
  if (action === 'rename') startRename(track)
  else if (action === 'up') editor.moveTrack(track.id, index - 1)
  else if (action === 'down') editor.moveTrack(track.id, index + 1)
  else if (action === 'delete') editor.removeTrack(track.id)
}

function startRename(track: Track) {
  editingTrack.value = track.id
  trackDraft.value = track.name
  void nextTick(() => document.querySelector<HTMLInputElement>(`[data-track-input="${track.id}"]`)?.select())
}

function finishRename(track: Track, save: boolean) {
  if (save && trackDraft.value.trim() && trackDraft.value.trim() !== track.name) editor.updateTrack(track.id, { name: trackDraft.value.trim().slice(0, 60) })
  editingTrack.value = null
}

// ---------------------------------------------------------------- clip appearance

function clipColor(item: TimelineItem) {
  return TRACK_COLORS[item.kind]
}

function clipIcon(item: TimelineItem) {
  return { video: Film, image: ImageIcon, audio: AudioLines, text: Type, shape: Shapes }[item.kind]
}

function clipLabel(item: TimelineItem) {
  if (item.kind === 'text') return item.text?.content.replace(/\s+/g, ' ') || item.name
  return item.name
}

function keyframesOf(item: TimelineItem) {
  return item.keyframes?.length ? sortedKeyframes(item) : []
}

function visibleRange(item: TimelineItem) {
  const from = Math.max(0, (viewStart.value - item.start) * zoom.value)
  const to = Math.min(item.duration * zoom.value, (viewEnd.value - item.start) * zoom.value)
  return { from, to: Math.max(from, to) }
}

function assetOf(item: TimelineItem) {
  return item.assetId ? editor.assetsById.get(item.assetId) : undefined
}

// ---------------------------------------------------------------- resizing the panel

function startResize(event: PointerEvent) {
  event.preventDefault()
  const startY = event.clientY
  const startHeight = props.height
  bindDrag(move => emit('resize', clamp(startHeight + startY - move.clientY, 180, window.innerHeight - 260)), () => {}, 'row-resize')
}

let unregister: (() => void) | null = null
let resizeObserver: ResizeObserver | null = null

onMounted(() => {
  unregister = registerDropTarget(() => scroller.value, (payload, clientX, clientY) => {
    if (!computeDrop(payload, clientX, clientY)) return false
    void handleDrop(payload, clientX, clientY)
    return true
  })
  resizeObserver = new ResizeObserver(onScroll)
  if (scroller.value) resizeObserver.observe(scroller.value)
  onScroll()
})

onBeforeUnmount(() => {
  unregister?.()
  resizeObserver?.disconnect()
  stopAutoScroll()
})

defineExpose({ zoomToFit, setZoom })
</script>

<template>
  <section class="surface relative flex shrink-0 flex-col" :style="{ height: `${height}px` }">
    <div class="absolute inset-x-0 -top-2 z-30 h-2 cursor-row-resize" @pointerdown="startResize" />

    <div class="flex h-12 shrink-0 items-center gap-0.5 border-b border-line px-2.5">
      <UiIconButton label="Undo" shortcut="Ctrl+Z" :disabled="!editor.canUndo" @click="editor.undo()"><Undo2 class="size-4" /></UiIconButton>
      <UiIconButton label="Redo" shortcut="Ctrl+Shift+Z" :disabled="!editor.canRedo" @click="editor.redo()"><Redo2 class="size-4" /></UiIconButton>
      <span class="mx-2 h-5 w-px bg-line" />
      <UiIconButton label="Split" shortcut="S" @click="editor.splitAt()"><Scissors class="size-4" /></UiIconButton>
      <UiIconButton label="Trim start to playhead" shortcut="Q" :disabled="!editor.selectedItem" @click="editor.trimToPlayhead('start')"><ArrowLeftToLine class="size-4" /></UiIconButton>
      <UiIconButton label="Trim end to playhead" shortcut="W" :disabled="!editor.selectedItem" @click="editor.trimToPlayhead('end')"><ArrowRightToLine class="size-4" /></UiIconButton>
      <UiIconButton label="Duplicate" shortcut="Ctrl+D" :disabled="!editor.selectedIds.length" @click="editor.duplicateSelected()"><Copy class="size-4" /></UiIconButton>
      <UiIconButton label="Delete" shortcut="Del" :disabled="!editor.selectedIds.length" @click="editor.deleteSelected()"><Trash2 class="size-4" /></UiIconButton>
      <span class="mx-2 h-5 w-px bg-line" />
      <UiIconButton label="Add or remove keyframe" shortcut="K" :disabled="!editor.selectedItem || editor.selectedItem.kind === 'audio'" @click="editor.toggleKeyframeAtPlayhead()"><Diamond class="size-4" /></UiIconButton>
      <UiIconButton label="Transitions" @click="emit('openPanel', 'transitions')"><Blend class="size-4" /></UiIconButton>
      <div class="ml-auto flex items-center gap-1">
        <UiIconButton :label="editor.snapping ? 'Snapping on' : 'Snapping off'" :active="editor.snapping" @click="editor.snapping = !editor.snapping"><Magnet class="size-4" :class="editor.snapping && 'text-accent'" /></UiIconButton>
        <span class="mx-2 h-5 w-px bg-line" />
        <UiIconButton label="Zoom out" shortcut="-" @click="setZoom(zoom / 1.4)"><ZoomOut class="size-4" /></UiIconButton>
        <UiSlider v-model="zoomSlider" class="w-28" :min="Math.log(2)" :max="Math.log(600)" :step="0.01" />
        <UiIconButton label="Zoom in" shortcut="=" @click="setZoom(zoom * 1.4)"><ZoomIn class="size-4" /></UiIconButton>
        <UiIconButton label="Fit timeline" shortcut="Shift+Z" @click="zoomToFit"><Maximize2 class="size-4" /></UiIconButton>
      </div>
    </div>

    <div class="flex min-h-0 flex-1">
      <!-- Track headers -->
      <div class="flex shrink-0 flex-col border-r border-line" :style="{ width: `${HEADER_W}px` }">
        <div class="flex shrink-0 items-center border-b border-line px-2" :style="{ height: `${RULER_H}px` }">
          <UiMenu :items="addTrackItems" align="start" @select="kind => editor.addTrack(kind as TrackKind)">
            <button class="flex h-7 items-center gap-1.5 rounded-lg px-2 text-xs font-medium text-muted hover:bg-hover hover:text-fg"><Plus class="size-4" />Add track</button>
          </UiMenu>
        </div>
        <div ref="headerColumn" class="relative min-h-0 flex-1 overflow-hidden">
          <div class="relative" :style="{ height: `${rowsHeight}px` }">
            <div
              v-for="row in rows" :key="row.track.id"
              class="group absolute inset-x-0 flex cursor-grab items-center gap-1.5 rounded-sm px-2 transition-colors"
              :style="{ top: `${row.top}px`, height: `${row.height}px` }"
              :class="[row.track.hidden && 'opacity-50', trackDrag?.id === row.track.id ? 'bg-hover opacity-60' : 'hover:bg-hover/40']"
              title="Drag to reorder"
              @pointerdown="startTrackDrag($event, row.track)"
            >
              <span class="relative size-4 shrink-0">
                <component :is="trackIcons[row.track.kind]" class="absolute inset-0 size-4 text-muted transition-opacity group-hover:opacity-0" />
                <GripVertical class="absolute inset-0 size-4 text-muted opacity-0 transition-opacity group-hover:opacity-100" />
              </span>
              <input
                v-if="editingTrack === row.track.id" v-model="trackDraft" :data-track-input="row.track.id"
                class="h-7 min-w-0 flex-1 rounded-md border border-accent bg-raised px-1.5 text-xs text-fg outline-none"
                @blur="finishRename(row.track, true)" @keydown.enter.prevent="finishRename(row.track, true)" @keydown.esc.prevent="finishRename(row.track, false)"
              >
              <button v-else data-track-grab class="min-w-0 flex-1 cursor-grab truncate text-left text-xs text-muted hover:text-fg" @dblclick="startRename(row.track)">{{ row.track.name }}</button>
              <button v-if="row.track.kind !== 'audio'" class="grid size-6 place-items-center rounded-md text-faint hover:bg-hover hover:text-fg" :aria-label="row.track.hidden ? 'Show track' : 'Hide track'" :title="row.track.hidden ? 'Show track' : 'Hide track'" @click="editor.updateTrack(row.track.id, { hidden: !row.track.hidden })"><EyeOff v-if="row.track.hidden" class="size-3.5 text-warn" /><Eye v-else class="size-3.5" /></button>
              <button v-if="row.track.kind !== 'text'" class="grid size-6 place-items-center rounded-md text-faint hover:bg-hover hover:text-fg" :aria-label="row.track.muted ? 'Unmute track' : 'Mute track'" :title="row.track.muted ? 'Unmute track' : 'Mute track'" @click="editor.updateTrack(row.track.id, { muted: !row.track.muted })"><VolumeX v-if="row.track.muted" class="size-3.5 text-warn" /><Volume2 v-else class="size-3.5" /></button>
              <button class="grid size-6 place-items-center rounded-md text-faint hover:bg-hover hover:text-fg" :aria-label="row.track.locked ? 'Unlock track' : 'Lock track'" :title="row.track.locked ? 'Unlock track' : 'Lock track'" @click="editor.updateTrack(row.track.id, { locked: !row.track.locked })"><Lock v-if="row.track.locked" class="size-3.5 text-warn" /><Unlock v-else class="size-3.5" /></button>
              <UiMenu :items="trackMenu" align="start" @select="action => trackAction(row.track, action)">
                <button class="grid size-5 place-items-center rounded text-faint opacity-0 hover:bg-hover hover:text-fg group-hover:opacity-100" aria-label="Track options"><MoreHorizontal class="size-3.5" /></button>
              </UiMenu>
            </div>
            <div v-if="trackDropLine !== null" class="pointer-events-none absolute inset-x-1 z-10 h-0.5 -translate-y-1/2 rounded bg-accent" :style="{ top: `${trackDropLine}px` }" />
          </div>
        </div>
      </div>

      <!-- Tracks -->
      <UiContextMenu :items="contextItems" @select="onContextAction">
        <div ref="scroller" class="no-scrollbar-x relative min-w-0 flex-1 overflow-auto" @scroll="onScroll" @wheel="onWheel" @contextmenu.capture="onContextMenu">
          <div class="relative" :style="{ width: `${contentWidth}px`, height: `${RULER_H + rowsHeight}px` }">
            <div class="sticky top-0 z-20 cursor-ew-resize border-b border-line bg-panel" :style="{ height: `${RULER_H}px` }" @pointerdown="startScrub">
              <template v-for="tick in ruler.ticks" :key="tick.time">
                <div class="absolute bottom-0 w-px" :class="tick.major ? 'h-2.5 bg-line-strong' : 'h-1.5 bg-line'" :style="{ left: `${x(tick.time)}px` }" />
                <span v-if="tick.major" class="absolute top-1 pl-1 text-[11px] tabular-nums text-faint" :style="{ left: `${x(tick.time)}px` }">{{ rulerLabel(tick.time, ruler.major) }}</span>
              </template>
            </div>

            <div class="absolute inset-x-0 bottom-0" :style="{ top: `${RULER_H}px` }" @pointerdown.self="startArea">
              <div v-for="row in rows" :key="row.track.id" class="group absolute inset-x-0 rounded-sm" :class="[row.track.locked && 'bg-[repeating-linear-gradient(135deg,transparent_0_6px,rgba(255,255,255,0.025)_6px_12px)]', dropHint?.kind === 'track' && dropHint.trackId === row.track.id ? 'bg-accent/[.06]' : 'bg-app/40', trackDrag?.id === row.track.id && 'opacity-50']" :style="{ top: `${row.top}px`, height: `${row.height}px` }" @pointerdown.self="startArea">
                <div
                  v-for="item in visibleItems(row.track)" :key="item.id"
                  :data-item-id="item.id"
                  class="group/clip absolute top-0 h-full overflow-hidden rounded-md border"
                  :class="[
                    dropHint?.kind === 'clip' && dropHint.itemId === item.id ? 'z-10 border-[#c084fc] shadow-[0_0_0_2px_rgba(192,132,252,0.55)]' : editor.selectedIds.includes(item.id) ? 'z-10 border-white shadow-[0_0_0_1px_rgba(255,255,255,0.6)]' : 'border-transparent hover:border-white/30',
                    (item.locked || row.track.locked) ? 'cursor-not-allowed' : 'cursor-grab',
                    item.hidden && 'opacity-40'
                  ]"
                  :style="{ left: `${x(item.start)}px`, width: `${Math.max(2, item.duration * zoom)}px`, background: `color-mix(in srgb, ${clipColor(item)} ${item.kind === 'video' || item.kind === 'image' ? 30 : 62}%, #17171b)` }"
                  @pointerdown="startClip($event, item, row.track)"
                >
                  <ClipFilmstrip v-if="item.kind === 'video' && assetOf(item)" :asset="assetOf(item)!" :source-in="item.sourceIn ?? 0" :speed="item.speed || 1" :zoom="zoom" :height="row.height - 2" v-bind="visibleRange(item)" />
                  <div v-else-if="item.kind === 'image' && assetOf(item)" class="pointer-events-none absolute inset-0 bg-repeat-x opacity-90" :style="{ backgroundImage: `url(&quot;${fileUrl(assetOf(item)!.previewPath ?? assetOf(item)!.path)}&quot;)`, backgroundSize: 'auto 100%' }" />
                  <ClipWaveform v-else-if="item.kind === 'audio' && assetOf(item)" :asset="assetOf(item)!" :source-in="item.sourceIn ?? 0" :speed="item.speed || 1" :zoom="zoom" :volume="item.muted ? 0 : item.volume ?? 1" :color="'rgba(120,230,190,0.9)'" v-bind="visibleRange(item)" />
                  <div v-if="item.kind === 'video' || item.kind === 'image'" class="pointer-events-none absolute inset-0 bg-gradient-to-b from-black/45 via-transparent to-transparent" />
                  <div class="pointer-events-none relative flex h-5 items-center gap-1.5 px-2 pt-0.5 text-xs font-medium text-white [text-shadow:0_1px_2px_rgba(0,0,0,0.6)]">
                    <component :is="clipIcon(item)" class="size-3.5 shrink-0 opacity-90" />
                    <span class="truncate">{{ clipLabel(item) }}</span>
                    <span v-if="item.speed && item.speed !== 1" class="shrink-0 rounded bg-black/50 px-1 tabular-nums">{{ item.speed }}×</span>
                    <VolumeX v-if="item.muted" class="size-3 shrink-0 text-warn" />
                  </div>
                  <div v-if="assetOf(item) === undefined && item.assetId" class="pointer-events-none absolute inset-x-1 bottom-1 truncate text-[11px] text-danger">Media missing</div>
                  <div v-if="(item.fadeIn ?? 0) > 0" class="pointer-events-none absolute left-0 top-0 h-full bg-gradient-to-r from-black/50 to-transparent" :style="{ width: `${(item.fadeIn ?? 0) * zoom}px` }" />
                  <div v-if="(item.fadeOut ?? 0) > 0" class="pointer-events-none absolute right-0 top-0 h-full bg-gradient-to-l from-black/50 to-transparent" :style="{ width: `${(item.fadeOut ?? 0) * zoom}px` }" />
                  <button
                    v-for="keyframe in keyframesOf(item)" :key="keyframe.id"
                    class="absolute bottom-0.5 z-30 size-2 -translate-x-1/2 rotate-45 border border-black/60 cursor-ew-resize"
                    :class="Math.abs(editor.currentTime - item.start - keyframe.time) <= editor.keyframeTolerance ? 'bg-accent' : 'bg-white/85'"
                    :style="{ left: `${keyframe.time * zoom}px` }"
                    :aria-label="`Keyframe at ${keyframe.time.toFixed(2)}s`"
                    @pointerdown="startKeyframe($event, item, keyframe.id, keyframe.time)"
                  />
                  <div
                    v-if="hasEffect(item)"
                    class="group/fx absolute bottom-0.5 z-20 flex h-4 cursor-grab items-center gap-1 overflow-hidden rounded border border-[#c084fc]/80 bg-[#7c3aed]/90 text-[11px] font-medium leading-none text-white shadow-sm"
                    :style="{ left: `${effectWindow(item).start * zoom}px`, width: `${Math.max(4, effectWindow(item).duration * zoom)}px` }"
                    :title="`${effectLabel(item)} · ${effectWindow(item).duration.toFixed(1)}s - drag to move, drag the edges to stretch`"
                    @pointerdown="startEffect($event, item, row.track, 'move')"
                  >
                    <Sparkles class="ml-1.5 size-3 shrink-0" />
                    <span class="min-w-0 flex-1 truncate">{{ effectLabel(item) }}</span>
                    <button v-if="!item.locked && !row.track.locked && effectWindow(item).duration * zoom > 44" class="mr-1.5 grid size-2.5 shrink-0 place-items-center rounded-sm opacity-0 hover:bg-white/25 group-hover/fx:opacity-100" aria-label="Remove effect" @pointerdown.stop @click.stop="removeEffect(item)"><X class="size-2" /></button>
                    <template v-if="!item.locked && !row.track.locked">
                      <div class="absolute inset-y-0 left-0 w-1.5 cursor-ew-resize hover:bg-white/60" @pointerdown="startEffect($event, item, row.track, 'start')" />
                      <div class="absolute inset-y-0 right-0 w-1.5 cursor-ew-resize hover:bg-white/60" @pointerdown="startEffect($event, item, row.track, 'end')" />
                    </template>
                  </div>
                  <template v-if="!item.locked && !row.track.locked">
                    <div class="absolute inset-y-0 left-0 z-10 w-2 cursor-ew-resize rounded-l-[4px] bg-white/0 transition-colors hover:bg-white/80" :class="editor.selectedIds.includes(item.id) && 'bg-white/70'" @pointerdown="startTrim($event, item, 'start')" />
                    <div class="absolute inset-y-0 right-0 z-10 w-2 cursor-ew-resize rounded-r-[4px] bg-white/0 transition-colors hover:bg-white/80" :class="editor.selectedIds.includes(item.id) && 'bg-white/70'" @pointerdown="startTrim($event, item, 'end')" />
                  </template>
                </div>

                <template v-for="seam in seamsByTrack.get(row.track.id) ?? []" :key="`seam-${seam.to.id}`">
                  <div
                    v-if="seam.kind !== 'none'"
                    class="group/tr absolute top-1/2 z-20 flex h-[54%] min-w-[14px] -translate-y-1/2 cursor-pointer items-center justify-center gap-1 overflow-hidden rounded-[4px] border text-[10px] font-medium text-white shadow-[0_1px_4px_rgba(0,0,0,0.45)] backdrop-blur-sm"
                    :class="(dropHint?.kind === 'seam' && dropHint.seam.to.id === seam.to.id) || editor.primaryId === seam.to.id || resizingSeam === seam.to.id ? 'border-white bg-accent/70' : 'border-accent/70 bg-[#1c2a45]/90 hover:bg-[#243656]/90'"
                    :style="{ left: `${x(seam.cut - seam.duration / 2)}px`, width: `${seam.duration * zoom}px` }"
                    :title="`${transitionLabel(seam)} - drag the edges to change the length`"
                    @pointerdown.stop
                    @click="openSeam(seam)"
                  >
                    <Blend class="size-3 shrink-0" />
                    <span v-if="seam.duration * zoom > 56 || resizingSeam === seam.to.id" class="truncate tabular-nums">{{ seam.duration.toFixed(1) }}s</span>
                    <button v-if="seamEditable(seam, row.track) && seam.duration * zoom > 64" class="absolute right-1.5 top-1/2 grid size-3.5 -translate-y-1/2 place-items-center rounded-sm opacity-0 hover:bg-white/25 group-hover/tr:opacity-100" aria-label="Remove transition" @pointerdown.stop @click.stop="editor.setTransition(seam.to.id, 'none')"><X class="size-2.5" /></button>
                    <template v-if="seamEditable(seam, row.track)">
                      <div class="absolute inset-y-0 left-0 w-1.5 cursor-ew-resize hover:bg-white/60" @pointerdown="startTransitionResize($event, seam, 'start')" />
                      <div class="absolute inset-y-0 right-0 w-1.5 cursor-ew-resize hover:bg-white/60" @pointerdown="startTransitionResize($event, seam, 'end')" />
                    </template>
                  </div>
                  <button
                    v-else
                    class="absolute top-1/2 z-20 grid size-5 -translate-x-1/2 -translate-y-1/2 place-items-center rounded border transition-[opacity,transform]"
                    :class="dropHint?.kind === 'seam' && dropHint.seam.to.id === seam.to.id ? 'scale-125 border-accent bg-accent text-accent-ink opacity-100' : drag.payload?.type === 'transition' ? 'border-accent bg-panel text-accent opacity-100' : 'border-line-strong bg-panel text-muted opacity-0 hover:opacity-100 group-hover:opacity-100'"
                    :style="{ left: `${x(seam.cut)}px` }"
                    :title="transitionLabel(seam)"
                    :aria-label="transitionLabel(seam)"
                    @pointerdown.stop
                    @click="openSeam(seam)"
                  >
                    <Plus class="size-3" />
                  </button>
                </template>
              </div>

              <div v-if="trackDropLine !== null" class="pointer-events-none absolute inset-x-0 z-30 h-0.5 -translate-y-1/2 bg-accent" :style="{ top: `${trackDropLine}px` }" />
              <div v-if="dropHint?.kind === 'track' && dropRow" class="pointer-events-none absolute z-20 rounded-[5px] border-2 border-dashed border-accent bg-accent/15" :style="{ left: `${x(dropHint.start)}px`, width: `${Math.max(6, dropHint.duration * zoom)}px`, top: `${dropRow.top}px`, height: `${dropRow.height}px` }" />
              <div v-else-if="dropHint?.kind === 'new'" class="pointer-events-none absolute z-20 h-0.5 rounded bg-accent" :style="{ left: `${x(dropHint.start)}px`, width: `${Math.max(40, dropHint.duration * zoom)}px`, top: `${dropHint.top}px` }" />
              <div v-if="marquee" class="pointer-events-none absolute z-30 border border-accent bg-accent/10" :style="{ left: `${Math.min(marquee.x0, marquee.x1)}px`, top: `${Math.min(marquee.y0, marquee.y1)}px`, width: `${Math.abs(marquee.x1 - marquee.x0)}px`, height: `${Math.abs(marquee.y1 - marquee.y0)}px` }" />
              <div v-if="!rows.length" class="pointer-events-none absolute inset-0 grid place-items-center text-xs text-faint">Drag media here or use the panels on the left</div>
            </div>

            <div v-if="snapLine !== null" class="pointer-events-none absolute bottom-0 top-0 z-30 w-px bg-[#f472b6]" :style="{ left: `${x(snapLine)}px` }" />
            <div class="pointer-events-none absolute bottom-0 top-0 z-40 w-px bg-accent" :style="{ transform: `translateX(${playheadX}px)` }">
              <div class="absolute -left-[5px] top-0 h-3 w-[11px] rounded-b-[3px] bg-accent" />
            </div>
          </div>
        </div>
      </UiContextMenu>
    </div>
  </section>
</template>
