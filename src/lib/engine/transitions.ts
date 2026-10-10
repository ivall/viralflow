import type { TimelineItem, Track, TransitionKind } from '../types'
import { brightnessMatrix, IDENTITY, mixMatrix, type ColorMatrix } from './color'
import { clamp, easeInOutCubic, easeOutCubic } from './math'

export const MIN_TRANSITION = 0.1
export const MAX_TRANSITION = 3
export const DEFAULT_TRANSITION = 0.5

export interface TransitionPreset {
  kind: Exclude<TransitionKind, 'none'>
  label: string
}

export const TRANSITIONS: TransitionPreset[] = [
  { kind: 'fade', label: 'Cross dissolve' },
  { kind: 'black', label: 'Dip to black' },
  { kind: 'white', label: 'Dip to white' },
  { kind: 'slide-left', label: 'Push left' },
  { kind: 'slide-right', label: 'Push right' },
  { kind: 'slide-up', label: 'Push up' },
  { kind: 'slide-down', label: 'Push down' },
  { kind: 'zoom', label: 'Zoom' },
  { kind: 'blur', label: 'Blur' },
  { kind: 'wipe-left', label: 'Wipe left' },
  { kind: 'wipe-right', label: 'Wipe right' },
  { kind: 'flash', label: 'Flash' },
  { kind: 'spin', label: 'Spin' }
]

export interface TransitionConnection {
  trackId: string
  from: TimelineItem
  to: TimelineItem
  kind: Exclude<TransitionKind, 'none'>
  cut: number
  start: number
  end: number
  duration: number
}

export interface Seam {
  trackId: string
  from: TimelineItem
  to: TimelineItem
  cut: number
  kind: TransitionKind
  duration: number
}

export interface TransitionState {
  dx: number
  dy: number
  scale: number
  rotation: number
  opacity: number
  blur: number
  matrix: ColorMatrix
  /** Visible horizontal span of the layer box as fractions [left, right]. */
  clipLeft: number
  clipRight: number
}

export const IDLE_TRANSITION: TransitionState = { dx: 0, dy: 0, scale: 1, rotation: 0, opacity: 1, blur: 0, matrix: IDENTITY, clipLeft: 0, clipRight: 1 }

export function isTransitionable(item: TimelineItem) {
  return (item.kind === 'video' || item.kind === 'image') && !item.hidden
}

function seamTolerance(fps: number) {
  return Math.max(0.002, 0.5 / Math.max(1, fps))
}

export function transitionDuration(to: TimelineItem, from: TimelineItem) {
  const requested = to.transition?.duration || DEFAULT_TRANSITION
  return clamp(Math.min(requested, from.duration, to.duration), MIN_TRANSITION, MAX_TRANSITION)
}

/** All cuts between touching visual clips of a track, with or without a transition. */
export function trackSeams(track: Track, fps: number): Seam[] {
  if (track.kind !== 'video') return []
  const tolerance = seamTolerance(fps)
  const items = track.items.filter(isTransitionable).sort((a, b) => a.start - b.start || a.id.localeCompare(b.id))
  const seams: Seam[] = []
  for (let index = 1; index < items.length; index++) {
    const from = items[index - 1]
    const to = items[index]
    if (Math.abs(from.start + from.duration - to.start) > tolerance) continue
    seams.push({ trackId: track.id, from, to, cut: to.start, kind: to.transition?.kind ?? 'none', duration: transitionDuration(to, from) })
  }
  return seams
}

export function trackConnections(track: Track, fps: number): TransitionConnection[] {
  if (track.hidden) return []
  return trackSeams(track, fps).flatMap(seam => seam.kind === 'none' ? [] : [{
    trackId: seam.trackId,
    from: seam.from,
    to: seam.to,
    kind: seam.kind as Exclude<TransitionKind, 'none'>,
    cut: seam.cut,
    duration: seam.duration,
    start: seam.cut - seam.duration / 2,
    end: seam.cut + seam.duration / 2
  }])
}

/** Drops transitions that no longer sit on a cut and clamps their length. */
export function repairTransitions(track: Track, fps: number) {
  const seams = new Map(trackSeams(track, fps).map(seam => [seam.to.id, seam]))
  for (const item of track.items) {
    if (!item.transition || item.transition.kind === 'none') { delete item.transition; continue }
    const seam = seams.get(item.id)
    if (!seam) { delete item.transition; continue }
    item.transition = { kind: item.transition.kind, duration: transitionDuration(item, seam.from) }
  }
}

export function resolveTransition(connection: TransitionConnection | undefined, itemId: string, time: number): TransitionState {
  if (!connection || time < connection.start || time > connection.end) return IDLE_TRANSITION
  const incoming = connection.to.id === itemId
  if (!incoming && connection.from.id !== itemId) return IDLE_TRANSITION
  const p = clamp((time - connection.start) / connection.duration, 0, 1)
  const e = easeInOutCubic(p)
  const s = easeOutCubic(p)
  const state: TransitionState = { ...IDLE_TRANSITION }
  switch (connection.kind) {
    case 'fade':
      if (incoming) state.opacity = e
      break
    case 'black':
    case 'white': {
      const color: [number, number, number] = connection.kind === 'black' ? [0, 0, 0] : [1, 1, 1]
      if (incoming) {
        if (p < 0.5) state.opacity = 0
        else state.matrix = mixMatrix(color, 1 - (p - 0.5) / 0.5)
      } else if (p < 0.5) state.matrix = mixMatrix(color, p / 0.5)
      else state.opacity = 0
      break
    }
    case 'slide-left': state.dx = incoming ? (1 - e) * 100 : -e * 100; break
    case 'slide-right': state.dx = incoming ? -(1 - e) * 100 : e * 100; break
    case 'slide-up': state.dy = incoming ? (1 - e) * 100 : -e * 100; break
    case 'slide-down': state.dy = incoming ? -(1 - e) * 100 : e * 100; break
    case 'zoom':
      if (incoming) { state.opacity = s; state.scale = 1.3 - 0.3 * s; state.blur = (1 - s) * 8 }
      else state.scale = 1 + 0.2 * e
      break
    case 'blur':
      if (incoming) { state.opacity = e; state.blur = (1 - s) * 20 }
      else state.blur = e * 14
      break
    case 'wipe-left':
      if (incoming) state.clipLeft = 1 - e
      break
    case 'wipe-right':
      if (incoming) state.clipRight = e
      break
    case 'flash':
      if (incoming) state.opacity = e
      state.matrix = brightnessMatrix(Math.sin(e * Math.PI) * (incoming ? 0.8 : 0.6))
      break
    case 'spin':
      if (incoming) { state.opacity = s; state.scale = 0.6 + 0.4 * s; state.rotation = (1 - s) * 30 }
      else { state.scale = 1 + 0.2 * e; state.rotation = -e * 20 }
      break
  }
  return state
}
