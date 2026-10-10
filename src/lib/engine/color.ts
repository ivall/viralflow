import type { Adjust } from '../types'
import { clamp } from './math'

/**
 * 4×5 row-major colour matrix in the same layout as SVG feColorMatrix:
 * R' = m0·R + m1·G + m2·B + m3·A + m4 (channels and offsets in 0..1).
 * The Rust exporter (engine/color.rs) builds identical matrices.
 */
export type ColorMatrix = number[]

export const IDENTITY: ColorMatrix = [
  1, 0, 0, 0, 0,
  0, 1, 0, 0, 0,
  0, 0, 1, 0, 0,
  0, 0, 0, 1, 0
]

export const NEUTRAL_ADJUST: Adjust = { exposure: 0, brightness: 0, contrast: 0, saturation: 0, temperature: 0, tint: 0, hue: 0, vignette: 0 }

/** Returns a ∘ b, i.e. b is applied first, then a. */
export function multiply(a: ColorMatrix, b: ColorMatrix): ColorMatrix {
  const out = new Array<number>(20)
  for (let row = 0; row < 4; row++) {
    for (let col = 0; col < 5; col++) {
      let value = col === 4 ? a[row * 5 + 4] : 0
      for (let k = 0; k < 4; k++) value += a[row * 5 + k] * b[k * 5 + col]
      out[row * 5 + col] = value
    }
  }
  return out
}

export function isIdentity(matrix: ColorMatrix) {
  return matrix.every((value, index) => Math.abs(value - IDENTITY[index]) < 1e-6)
}

function scaleRgb(r: number, g: number, b: number): ColorMatrix {
  return [r, 0, 0, 0, 0, 0, g, 0, 0, 0, 0, 0, b, 0, 0, 0, 0, 0, 1, 0]
}

function offsetRgb(value: number): ColorMatrix {
  return [1, 0, 0, 0, value, 0, 1, 0, 0, value, 0, 0, 1, 0, value, 0, 0, 0, 1, 0]
}

function contrastMatrix(k: number): ColorMatrix {
  const offset = 0.5 * (1 - k)
  return [k, 0, 0, 0, offset, 0, k, 0, 0, offset, 0, 0, k, 0, offset, 0, 0, 0, 1, 0]
}

/** CSS saturate() matrix. */
function saturateMatrix(s: number): ColorMatrix {
  return [
    0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s, 0, 0,
    0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s, 0, 0,
    0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s, 0, 0,
    0, 0, 0, 1, 0
  ]
}

/** CSS hue-rotate() matrix. */
function hueMatrix(degrees: number): ColorMatrix {
  const angle = (degrees * Math.PI) / 180
  const c = Math.cos(angle)
  const s = Math.sin(angle)
  return [
    0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928, 0, 0,
    0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.14, 0.072 - c * 0.072 - s * 0.283, 0, 0,
    0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072, 0, 0,
    0, 0, 0, 1, 0
  ]
}

/** Mixes the RGB towards a solid colour: out = in·(1-a) + color·a. */
export function mixMatrix(color: [number, number, number], amount: number): ColorMatrix {
  const a = clamp(amount, 0, 1)
  const k = 1 - a
  return [k, 0, 0, 0, color[0] * a, 0, k, 0, 0, color[1] * a, 0, 0, k, 0, color[2] * a, 0, 0, 0, 1, 0]
}

export function brightnessMatrix(offset: number) {
  return offsetRgb(offset)
}

export function adjustMatrix(adjust: Partial<Adjust> | undefined): ColorMatrix {
  const a = { ...NEUTRAL_ADJUST, ...adjust }
  let m = IDENTITY
  if (a.exposure) { const k = Math.pow(2, a.exposure * 1.5); m = multiply(scaleRgb(k, k, k), m) }
  if (a.brightness) m = multiply(offsetRgb(a.brightness * 0.3), m)
  if (a.contrast) m = multiply(contrastMatrix(a.contrast >= 0 ? 1 + a.contrast * 1.5 : 1 + a.contrast), m)
  if (a.saturation) m = multiply(saturateMatrix(Math.max(0, 1 + a.saturation)), m)
  if (a.hue) m = multiply(hueMatrix(a.hue * 180), m)
  if (a.temperature) m = multiply(scaleRgb(1 + 0.15 * a.temperature, 1 + 0.03 * a.temperature, 1 - 0.15 * a.temperature), m)
  if (a.tint) m = multiply(scaleRgb(1 + 0.05 * a.tint, 1 - 0.12 * a.tint, 1 + 0.05 * a.tint), m)
  return m
}

export function hexToRgb(hex: string): [number, number, number] {
  const value = hex.trim().replace('#', '')
  if (!/^[\da-f]{6}$/i.test(value)) return [1, 1, 1]
  const int = Number.parseInt(value, 16)
  return [((int >> 16) & 255) / 255, ((int >> 8) & 255) / 255, (int & 255) / 255]
}

export function rgba(hex: string, opacity: number) {
  const [r, g, b] = hexToRgb(hex)
  return `rgba(${Math.round(r * 255)}, ${Math.round(g * 255)}, ${Math.round(b * 255)}, ${clamp(opacity, 0, 1)})`
}

export function contrastAndSaturation(contrast: number, saturation: number): ColorMatrix {
  return multiply(saturateMatrix(saturation), contrastMatrix(contrast))
}
