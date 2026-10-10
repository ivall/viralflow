import type { EffectKind } from '../types'
import { contrastAndSaturation, brightnessMatrix, IDENTITY, type ColorMatrix } from './color'
import { clamp, hash01 } from './math'

export interface EffectState {
  dx: number
  dy: number
  scale: number
  blur: number
  matrix: ColorMatrix
  /** Film grain strength 0..1. */
  grain: number
  scanlines: boolean
  /** Horizontal band displacement in % of the layer width, one entry per band (empty when idle). */
  glitch: number[]
}

const IDLE: EffectState = { dx: 0, dy: 0, scale: 1, blur: 0, matrix: IDENTITY, grain: 0, scanlines: false, glitch: [] }

export const GLITCH_BANDS = 6

export const EFFECTS: Array<{ kind: EffectKind; label: string; description: string }> = [
  { kind: 'none', label: 'None', description: 'No effect' },
  { kind: 'punch', label: 'Punch', description: 'Quick scale hit at the start' },
  { kind: 'shake', label: 'Shake', description: 'Decaying camera shake' },
  { kind: 'zoom-in', label: 'Push in', description: 'Slow zoom in across the clip' },
  { kind: 'zoom-out', label: 'Pull out', description: 'Slow zoom out across the clip' },
  { kind: 'flash', label: 'Flash', description: 'Bright flash at the start' },
  { kind: 'blur', label: 'Blur', description: 'Soft blur over the whole clip' },
  { kind: 'vhs', label: 'VHS', description: 'Grain, scanlines and faded colour' },
  { kind: 'glitch', label: 'Glitch', description: 'Digital band displacement' }
]

export function resolveEffect(effect: EffectKind | undefined, elapsed: number, duration: number): EffectState {
  if (!effect || effect === 'none') return IDLE
  const t = Math.max(0, elapsed)
  switch (effect) {
    case 'punch': {
      const p = clamp(t / 0.32, 0, 1)
      if (p >= 1) return IDLE
      return { ...IDLE, scale: 1 + 0.12 * (1 - p) ** 2 * Math.cos(p * Math.PI * 2) }
    }
    case 'shake': {
      const p = clamp(t / 0.45, 0, 1)
      if (p >= 1) return IDLE
      const envelope = (1 - p) ** 2
      return { ...IDLE, dx: Math.sin(t * 78) * 1.2 * envelope, dy: Math.cos(t * 62) * 0.7 * envelope }
    }
    case 'zoom-in':
      return { ...IDLE, scale: 1 + 0.15 * clamp(t / Math.max(0.01, duration), 0, 1) }
    case 'zoom-out':
      return { ...IDLE, scale: 1.15 - 0.15 * clamp(t / Math.max(0.01, duration), 0, 1) }
    case 'flash': {
      const p = clamp(t / 0.35, 0, 1)
      if (p >= 1) return IDLE
      return { ...IDLE, matrix: brightnessMatrix(0.6 * (1 - p) ** 2) }
    }
    case 'blur':
      return { ...IDLE, blur: 10 }
    case 'vhs':
      return { ...IDLE, matrix: contrastAndSaturation(1.18, 0.78), grain: 0.09, scanlines: true }
    case 'glitch': {
      const slot = Math.floor(t * 8)
      const burst = hash01(slot) > 0.6
      if (!burst) return IDLE
      const bands = Array.from({ length: GLITCH_BANDS }, (_, band) => (hash01(slot * 31 + band) - 0.5) * 6)
      return { ...IDLE, dx: (hash01(slot + 7) - 0.5) * 1.5, glitch: bands }
    }
    default:
      return IDLE
  }
}
