import type { Easing, Keyframe, TimelineItem, Transform } from '../types'
import { applyEasing, clamp, EPSILON, lerp, uid } from './math'

export const DEFAULT_TRANSFORM: Transform = { x: 50, y: 50, scale: 1, rotation: 0, opacity: 1 }
export const MIN_SCALE = 0.05
export const MAX_SCALE = 10

export function normalizeTransform(value: Partial<Transform> | undefined): Transform {
  const scale = value?.scale ?? 1
  return {
    x: clamp(Number.isFinite(value?.x) ? value!.x! : 50, -100, 200),
    y: clamp(Number.isFinite(value?.y) ? value!.y! : 50, -100, 200),
    scale: clamp(scale > 0 ? scale : 1, MIN_SCALE, MAX_SCALE),
    rotation: Number.isFinite(value?.rotation) ? value!.rotation! : 0,
    opacity: clamp(Number.isFinite(value?.opacity) ? value!.opacity! : 1, 0, 1)
  }
}

export function sortedKeyframes(item: Pick<TimelineItem, 'duration' | 'keyframes'>): Keyframe[] {
  const duration = Math.max(0, item.duration)
  return [...(item.keyframes ?? [])]
    .filter(keyframe => Number.isFinite(keyframe.time))
    .map(keyframe => ({ ...keyframe, time: clamp(keyframe.time, 0, duration) }))
    .sort((left, right) => left.time - right.time || left.id.localeCompare(right.id))
    .filter((keyframe, index, list) => index === list.length - 1 || Math.abs(list[index + 1].time - keyframe.time) > EPSILON)
}

function interpolate(left: Transform, right: Transform, progress: number, easing: Easing): Transform {
  const t = applyEasing(progress, easing)
  return {
    x: lerp(left.x, right.x, t),
    y: lerp(left.y, right.y, t),
    scale: lerp(left.scale, right.scale, t),
    rotation: lerp(left.rotation, right.rotation, t),
    opacity: lerp(left.opacity, right.opacity, t)
  }
}

/** Transform of a clip at `elapsed` seconds from its start, keyframes included. */
export function resolveTransform(item: Pick<TimelineItem, 'duration' | 'keyframes' | 'transform'>, elapsed: number): Transform {
  const base = normalizeTransform(item.transform)
  const keyframes = sortedKeyframes(item)
  if (!keyframes.length) return base
  const time = clamp(elapsed, 0, Math.max(0, item.duration))
  const first = keyframes[0]
  if (time <= first.time) return normalizeTransform(first.transform)
  for (let index = 1; index < keyframes.length; index++) {
    const right = keyframes[index]
    if (time > right.time) continue
    const left = keyframes[index - 1]
    const span = Math.max(EPSILON, right.time - left.time)
    return interpolate(normalizeTransform(left.transform), normalizeTransform(right.transform), (time - left.time) / span, right.easing)
  }
  return normalizeTransform(keyframes[keyframes.length - 1].transform)
}

export function keyframeAt(item: TimelineItem, elapsed: number, tolerance: number) {
  return sortedKeyframes(item).find(keyframe => Math.abs(keyframe.time - elapsed) <= tolerance) ?? null
}

function makeKeyframe(transform: Transform, time: number, existing?: Keyframe): Keyframe {
  return { id: existing?.id ?? uid('kf'), time, transform: { ...transform }, easing: existing?.easing ?? 'linear' }
}

export function toggleKeyframe(item: TimelineItem, elapsed: number, tolerance: number): Keyframe[] {
  const time = clamp(elapsed, 0, item.duration)
  const existing = keyframeAt(item, time, tolerance)
  if (existing) return sortedKeyframes(item).filter(keyframe => keyframe.id !== existing.id)
  return [...sortedKeyframes(item), makeKeyframe(resolveTransform(item, time), time)].sort((a, b) => a.time - b.time)
}

/** Writes a transform change into the keyframe at the playhead, creating it when needed. */
export function patchKeyframeAt(item: TimelineItem, elapsed: number, patch: Partial<Transform>, tolerance: number): Keyframe[] {
  const time = clamp(elapsed, 0, item.duration)
  const keyframes = sortedKeyframes(item)
  const existing = keyframes.find(keyframe => Math.abs(keyframe.time - time) <= tolerance)
  const state = existing ? normalizeTransform(existing.transform) : resolveTransform(item, time)
  const next = makeKeyframe(normalizeTransform({ ...state, ...patch }), existing?.time ?? time, existing)
  return [...keyframes.filter(keyframe => keyframe.id !== existing?.id), next].sort((a, b) => a.time - b.time)
}

export function setKeyframeEasing(item: TimelineItem, elapsed: number, easing: Easing, tolerance: number): Keyframe[] {
  return sortedKeyframes(item).map(keyframe => Math.abs(keyframe.time - elapsed) <= tolerance ? { ...keyframe, easing } : keyframe)
}

export function adjacentKeyframe(item: TimelineItem, elapsed: number, direction: -1 | 1, tolerance: number) {
  const keyframes = sortedKeyframes(item)
  return direction < 0
    ? [...keyframes].reverse().find(keyframe => keyframe.time < elapsed - tolerance) ?? null
    : keyframes.find(keyframe => keyframe.time > elapsed + tolerance) ?? null
}

/** Stretches keyframe times when a clip changes length because of a speed change. */
export function retimeKeyframes(keyframes: Keyframe[] | undefined, fromDuration: number, toDuration: number): Keyframe[] {
  if (!keyframes?.length) return []
  const ratio = toDuration / Math.max(EPSILON, fromDuration)
  return keyframes.map(keyframe => ({ ...keyframe, time: clamp(keyframe.time * ratio, 0, toDuration) }))
}

/**
 * Cuts the keyframes of `item` down to the window [from, from + duration] and
 * rebases them to the new clip start. Boundary keyframes are inserted so the
 * motion inside the window stays exactly the same.
 */
export function trimKeyframes(item: TimelineItem, from: number, duration: number): Keyframe[] {
  const keyframes = sortedKeyframes(item)
  if (!keyframes.length) return []
  const safeFrom = clamp(from, 0, item.duration)
  const safeDuration = clamp(duration, 0, item.duration - safeFrom)
  const end = safeFrom + safeDuration
  const result = keyframes
    .filter(keyframe => keyframe.time >= safeFrom - EPSILON && keyframe.time <= end + EPSILON)
    .map(keyframe => ({ ...keyframe, time: clamp(keyframe.time - safeFrom, 0, safeDuration) }))
  if (safeFrom > EPSILON && !result.some(keyframe => keyframe.time <= EPSILON)) {
    result.push(makeKeyframe(resolveTransform(item, safeFrom), 0))
  }
  if (end < item.duration - EPSILON && !result.some(keyframe => Math.abs(keyframe.time - safeDuration) <= EPSILON)) {
    result.push(makeKeyframe(resolveTransform(item, end), safeDuration))
  }
  return result.sort((a, b) => a.time - b.time)
}
