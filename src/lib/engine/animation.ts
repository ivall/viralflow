import type { AnimationInKind, AnimationOutKind, TimelineItem } from '../types'
import { clamp, easeInCubic, easeOutCubic, lerp } from './math'

/** Offsets are in % of the canvas, blur in project pixels, rotation in degrees. */
export interface MotionState {
  dx: number
  dy: number
  scale: number
  rotation: number
  opacity: number
  blur: number
  /** Fraction (0..1) of the layer revealed from the left edge; 1 = fully visible. */
  reveal: number
}

export const IDLE_MOTION: MotionState = { dx: 0, dy: 0, scale: 1, rotation: 0, opacity: 1, blur: 0, reveal: 1 }

export interface AnimationPreset<K extends string> {
  kind: K
  label: string
  duration: number
}

export const ANIMATIONS_IN: AnimationPreset<AnimationInKind>[] = [
  { kind: 'none', label: 'None', duration: 0 },
  { kind: 'fade', label: 'Fade in', duration: 0.4 },
  { kind: 'pop', label: 'Pop', duration: 0.3 },
  { kind: 'rise', label: 'Rise', duration: 0.4 },
  { kind: 'drop', label: 'Drop', duration: 0.4 },
  { kind: 'slide-left', label: 'Slide from right', duration: 0.4 },
  { kind: 'slide-right', label: 'Slide from left', duration: 0.4 },
  { kind: 'zoom-in', label: 'Zoom in', duration: 0.4 },
  { kind: 'zoom-out', label: 'Zoom out', duration: 0.4 },
  { kind: 'bounce', label: 'Bounce', duration: 0.45 },
  { kind: 'spin', label: 'Spin', duration: 0.5 },
  { kind: 'blur', label: 'Focus', duration: 0.4 },
  { kind: 'typewriter', label: 'Typewriter', duration: 0.8 }
]

export const ANIMATIONS_OUT: AnimationPreset<AnimationOutKind>[] = [
  { kind: 'none', label: 'None', duration: 0 },
  { kind: 'fade', label: 'Fade out', duration: 0.4 },
  { kind: 'shrink', label: 'Shrink', duration: 0.35 },
  { kind: 'grow', label: 'Grow', duration: 0.35 },
  { kind: 'sink', label: 'Sink', duration: 0.4 },
  { kind: 'lift', label: 'Lift', duration: 0.4 },
  { kind: 'slide-left', label: 'Slide left', duration: 0.4 },
  { kind: 'slide-right', label: 'Slide right', duration: 0.4 },
  { kind: 'spin', label: 'Spin', duration: 0.5 },
  { kind: 'blur', label: 'Blur', duration: 0.4 }
]

export function defaultInDuration(kind: AnimationInKind) {
  return ANIMATIONS_IN.find(preset => preset.kind === kind)?.duration ?? 0.4
}

export function defaultOutDuration(kind: AnimationOutKind) {
  return ANIMATIONS_OUT.find(preset => preset.kind === kind)?.duration ?? 0.4
}

/** Effective animation windows; in and out never overlap. */
export function animationWindows(item: Pick<TimelineItem, 'duration' | 'animIn' | 'animOut'>) {
  const inKind = item.animIn?.kind ?? 'none'
  const outKind = item.animOut?.kind ?? 'none'
  let inDuration = inKind === 'none' ? 0 : Math.max(0.05, item.animIn!.duration)
  let outDuration = outKind === 'none' ? 0 : Math.max(0.05, item.animOut!.duration)
  const total = inDuration + outDuration
  if (total > item.duration && total > 0) {
    const ratio = item.duration / total
    inDuration *= ratio
    outDuration *= ratio
  }
  return { inKind, outKind, inDuration, outDuration }
}

function combine(a: MotionState, b: MotionState): MotionState {
  return {
    dx: a.dx + b.dx,
    dy: a.dy + b.dy,
    scale: a.scale * b.scale,
    rotation: a.rotation + b.rotation,
    opacity: a.opacity * b.opacity,
    blur: a.blur + b.blur,
    reveal: Math.min(a.reveal, b.reveal)
  }
}

function inState(kind: AnimationInKind, progress: number): MotionState {
  const p = clamp(progress, 0, 1)
  const e = easeOutCubic(p)
  const appear = clamp(p / 0.25, 0, 1)
  const s = { ...IDLE_MOTION }
  switch (kind) {
    case 'fade': s.opacity = e; break
    case 'pop': {
      s.opacity = appear
      s.scale = p < 0.6 ? lerp(0.6, 1.08, easeOutCubic(p / 0.6)) : lerp(1.08, 1, easeOutCubic((p - 0.6) / 0.4))
      break
    }
    case 'rise': s.opacity = appear; s.dy = (1 - e) * 10; break
    case 'drop': s.opacity = appear; s.dy = -(1 - e) * 10; break
    case 'slide-left': s.opacity = appear; s.dx = (1 - e) * 35; break
    case 'slide-right': s.opacity = appear; s.dx = -(1 - e) * 35; break
    case 'zoom-in': s.opacity = e; s.scale = lerp(0.4, 1, e); break
    case 'zoom-out': s.opacity = e; s.scale = lerp(1.8, 1, e); break
    case 'bounce': {
      s.opacity = appear
      s.scale = p < 0.45
        ? lerp(0.5, 1.16, easeOutCubic(p / 0.45))
        : p < 0.75
          ? lerp(1.16, 0.95, easeOutCubic((p - 0.45) / 0.3))
          : lerp(0.95, 1, easeOutCubic((p - 0.75) / 0.25))
      break
    }
    case 'spin': s.opacity = e; s.scale = lerp(0.3, 1, e); s.rotation = (1 - e) * -180; break
    case 'blur': s.opacity = e; s.blur = (1 - e) * 24; break
    case 'typewriter': s.reveal = p; break
  }
  return s
}

function outState(kind: AnimationOutKind, progress: number): MotionState {
  const q = easeInCubic(progress)
  const fade = 1 - clamp((progress - 0.6) / 0.4, 0, 1)
  const s = { ...IDLE_MOTION }
  switch (kind) {
    case 'fade': s.opacity = 1 - q; break
    case 'shrink': s.opacity = 1 - q; s.scale = lerp(1, 0.3, q); break
    case 'grow': s.opacity = 1 - q; s.scale = lerp(1, 1.8, q); break
    case 'sink': s.opacity = fade; s.dy = q * 10; break
    case 'lift': s.opacity = fade; s.dy = -q * 10; break
    case 'slide-left': s.opacity = fade; s.dx = -q * 35; break
    case 'slide-right': s.opacity = fade; s.dx = q * 35; break
    case 'spin': s.opacity = 1 - q; s.scale = lerp(1, 0.3, q); s.rotation = q * 180; break
    case 'blur': s.opacity = 1 - q; s.blur = q * 24; break
  }
  return s
}

/** `next` keeps the caption of `previous` on screen, only the highlighted word moved on. */
function continuesCaption(previous: TimelineItem, next: TimelineItem) {
  return previous !== next && previous.kind === 'text' && next.kind === 'text'
    && (next.text?.highlightIndex ?? -1) > 0
    && previous.text?.content === next.text?.content
    && Math.abs(previous.start + previous.duration - next.start) < 0.02
}

/** Clips that show the same word-highlight caption, in order; just `item` for any other clip. */
export function captionRun(item: TimelineItem, siblings: TimelineItem[]): TimelineItem[] {
  if (item.kind !== 'text') return [item]
  const run = [item]
  while (true) {
    const previous = siblings.find(candidate => continuesCaption(candidate, run[0]))
    if (!previous) break
    run.unshift(previous)
  }
  while (true) {
    const next = siblings.find(candidate => continuesCaption(run.at(-1)!, candidate))
    if (!next) break
    run.push(next)
  }
  return run
}

export type MotionSpan = Pick<TimelineItem, 'start' | 'duration' | 'animIn' | 'animOut'>

/**
 * Time the in/out animations of a clip play over. A word-highlight caption is one
 * clip per word, but it animates as one text: in on its first word, out on its last.
 */
export function motionSpan(item: TimelineItem, siblings: TimelineItem[]): MotionSpan {
  const run = captionRun(item, siblings)
  if (run.length === 1) return item
  const first = run[0]
  const last = run.at(-1)!
  return { start: first.start, duration: last.start + last.duration - first.start, animIn: first.animIn, animOut: last.animOut }
}

/** Motion of a clip `elapsed` seconds after its start, caused by in/out animations. */
export function resolveMotion(item: Pick<TimelineItem, 'duration' | 'animIn' | 'animOut'>, elapsed: number): MotionState {
  const { inKind, outKind, inDuration, outDuration } = animationWindows(item)
  let state = IDLE_MOTION
  if (inKind !== 'none' && elapsed < inDuration) state = combine(state, inState(inKind, elapsed / inDuration))
  const outStart = item.duration - outDuration
  if (outKind !== 'none' && elapsed > outStart) state = combine(state, outState(outKind, (elapsed - outStart) / outDuration))
  return state
}
