import type { Easing } from '../types'

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

export function applyEasing(progress: number, easing: Easing) {
  const value = clamp(progress, 0, 1)
  switch (easing) {
    case 'ease-in': return value * value
    case 'ease-out': return 1 - (1 - value) * (1 - value)
    case 'ease-in-out': return value < 0.5 ? 2 * value * value : 1 - Math.pow(-2 * value + 2, 2) / 2
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
