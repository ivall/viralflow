import type { Bezier, Easing } from '../types'

export const EPSILON = 1e-4

export function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
}

export function lerp(from: number, to: number, progress: number) {
  return from + (to - from) * progress
}

export function easeOutCubic(value: number) {
  return 1 - Math.pow(1 - clamp(value, 0, 1), 3)
}

export function easeInCubic(value: number) {
  return Math.pow(clamp(value, 0, 1), 3)
}

export function easeInOutCubic(value: number) {
  const v = clamp(value, 0, 1)
  return v < 0.5 ? 4 * v * v * v : 1 - Math.pow(-2 * v + 2, 3) / 2
}

/** Bezier presets; the named curves that predate custom curves keep their own formulas. */
export const EASING_CURVES: Record<Exclude<Easing, 'custom'>, Bezier> = {
  linear: [0.25, 0.25, 0.75, 0.75],
  'ease-in': [0.42, 0, 1, 1],
  'ease-out': [0, 0, 0.58, 1],
  'ease-in-out': [0.42, 0, 0.58, 1],
  'back-in': [0.36, 0, 0.66, -0.56],
  'back-out': [0.34, 1.56, 0.64, 1],
  'back-in-out': [0.68, -0.6, 0.32, 1.6]
}

export function sanitizeBezier(value: unknown): Bezier | null {
  if (!Array.isArray(value) || value.length !== 4 || !value.every(Number.isFinite)) return null
  const [x1, y1, x2, y2] = value as number[]
  return [clamp(x1, 0, 1), clamp(y1, -1, 2), clamp(x2, 0, 1), clamp(y2, -1, 2)]
}

/**
 * y of a CSS cubic-bezier at x (0..1). Newton steps with a bisection fallback;
 * mirrored operation for operation in Rust (engine::cubic_bezier).
 */
export function cubicBezier(curve: Bezier, x: number) {
  const [x1, y1, x2, y2] = sanitizeBezier(curve) ?? EASING_CURVES.linear
  const cx = 3 * x1
  const bx = 3 * (x2 - x1) - cx
  const ax = 1 - cx - bx
  const cy = 3 * y1
  const by = 3 * (y2 - y1) - cy
  const ay = 1 - cy - by
  const sampleX = (t: number) => ((ax * t + bx) * t + cx) * t
  const target = clamp(x, 0, 1)
  let t = target
  let solved = false
  for (let index = 0; index < 8; index++) {
    const error = sampleX(t) - target
    if (Math.abs(error) < 1e-7) { solved = true; break }
    const slope = (3 * ax * t + 2 * bx) * t + cx
    if (Math.abs(slope) < 1e-6) break
    t -= error / slope
  }
  if (!solved) {
    let low = 0
    let high = 1
    t = target
    for (let index = 0; index < 60; index++) {
      const value = sampleX(t)
      if (Math.abs(value - target) < 1e-7) break
      if (target > value) low = t
      else high = t
      t = (high - low) / 2 + low
    }
  }
  return ((ay * t + by) * t + cy) * t
}

export function applyEasing(progress: number, easing: Easing, bezier?: Bezier) {
  const value = clamp(progress, 0, 1)
  switch (easing) {
    case 'ease-in': return value * value
    case 'ease-out': return 1 - (1 - value) * (1 - value)
    case 'ease-in-out': return value < 0.5 ? 2 * value * value : 1 - Math.pow(-2 * value + 2, 2) / 2
    case 'back-in': case 'back-out': case 'back-in-out': return cubicBezier(EASING_CURVES[easing], value)
    case 'custom': {
      const curve = sanitizeBezier(bezier)
      return curve ? cubicBezier(curve, value) : value
    }
    default: return value
  }
}

/** Deterministic pseudo random number in [0, 1) - mirrored in Rust (engine::hash01). */
export function hash01(seed: number) {
  let x = Math.imul((seed | 0) ^ 0x9e3779b9, 0x85ebca6b)
  x = Math.imul(x ^ (x >>> 13), 0xc2b2ae35)
  x ^= x >>> 16
  return (x >>> 0) / 4294967296
}

export function uid(prefix = 'id') {
  const random = globalThis.crypto?.randomUUID?.().slice(0, 8) ?? Math.random().toString(36).slice(2, 10)
  return `${prefix}-${Date.now().toString(36)}-${random}`
}

export function roundTo(value: number, step: number) {
  return Math.round(value / step) * step
}
