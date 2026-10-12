import type { BlendMode, Crop, Fit, MediaAsset, Project, TimelineItem, Track } from '../types'
import { resolveMotion } from './animation'
import { adjustMatrix, IDENTITY, isIdentity, multiply, type ColorMatrix } from './color'
import { resolveItemEffect } from './effects'
import { resolveTransform } from './keyframes'
import { clamp } from './math'
import { IDLE_TRANSITION, resolveTransition, trackConnections, type TransitionConnection } from './transitions'

export interface SourceRect { sx: number; sy: number; sw: number; sh: number }

export interface MediaLayout extends SourceRect {
  boxW: number
  boxH: number
}

export interface Layer {
  item: TimelineItem
  trackIndex: number
  boxW: number
  boxH: number
  source?: SourceRect
  /** Source time (seconds) for video layers. */
  mediaTime?: number
  cx: number
  cy: number
  scale: number
  rotation: number
  flipX: boolean
  flipY: boolean
  opacity: number
  blur: number
  blend: BlendMode
  radius: number
  matrix: ColorMatrix | null
  vignette: number
  clipLeft: number
  clipRight: number
  grain: number
  scanlines: boolean
  glitch: number[]
}

export interface SceneContext {
  assets: Map<string, MediaAsset>
  /** Size of a text item's box in project pixels. */
  measureText(item: TimelineItem): { width: number; height: number }
}

export const FULL_CROP: Crop = { x: 0, y: 0, w: 1, h: 1 }

export function isVisualKind(kind: TimelineItem['kind']) {
  return kind !== 'audio'
}

export function mediaLayout(sourceW: number, sourceH: number, crop: Crop | undefined, fit: Fit | undefined, focus: { x: number; y: number } | undefined, canvasW: number, canvasH: number): MediaLayout {
  const c = crop ?? FULL_CROP
  const sw = Math.max(1, sourceW)
  const sh = Math.max(1, sourceH)
  const cx = clamp(c.x, 0, 1) * sw
  const cy = clamp(c.y, 0, 1) * sh
  const cw = Math.max(1, Math.min(sw - cx, clamp(c.w, 0.01, 1) * sw))
  const ch = Math.max(1, Math.min(sh - cy, clamp(c.h, 0.01, 1) * sh))
  if (fit === 'fill') return { sx: cx, sy: cy, sw: cw, sh: ch, boxW: canvasW, boxH: canvasH }
  if (fit === 'cover') {
    const k = Math.max(canvasW / cw, canvasH / ch)
    const vw = Math.min(cw, canvasW / k)
    const vh = Math.min(ch, canvasH / k)
    const fx = clamp(focus?.x ?? 50, 0, 100) / 100
    const fy = clamp(focus?.y ?? 50, 0, 100) / 100
    return { sx: cx + (cw - vw) * fx, sy: cy + (ch - vh) * fy, sw: vw, sh: vh, boxW: canvasW, boxH: canvasH }
  }
  const k = Math.min(canvasW / cw, canvasH / ch)
  return { sx: cx, sy: cy, sw: cw, sh: ch, boxW: cw * k, boxH: ch * k }
}

export function shapeBox(item: TimelineItem, canvasW: number, canvasH: number) {
  const shape = item.shape
  const width = clamp(shape?.width ?? 30, 0.2, 400) / 100 * canvasW
  const height = shape?.kind === 'circle' && shape.width === shape.height
    ? width
    : clamp(shape?.height ?? 20, 0.1, 400) / 100 * canvasH
  return { width, height }
}

/** Seconds into the source file shown at timeline `time`. */
export function mediaTimeAt(item: TimelineItem, time: number, fps: number, assetDuration?: number) {
  const speed = item.speed || 1
  const elapsed = clamp(time - item.start, 0, Math.max(0, item.duration - 1 / Math.max(1, fps)))
  let value = (item.sourceIn ?? 0) + elapsed * speed
  if (assetDuration && assetDuration > 0) value = Math.min(value, Math.max(0, assetDuration - 0.5 / Math.max(1, fps)))
  return Math.max(0, value)
}

export function itemBox(project: Project, item: TimelineItem, context: SceneContext): (MediaLayout | { boxW: number; boxH: number; sx?: undefined }) | null {
  if (item.kind === 'video' || item.kind === 'image') {
    const asset = item.assetId ? context.assets.get(item.assetId) : undefined
    if (!asset?.width || !asset.height) return null
    return mediaLayout(asset.width, asset.height, item.crop, item.fit ?? 'contain', item.focus, project.width, project.height)
  }
  if (item.kind === 'shape') {
    const { width, height } = shapeBox(item, project.width, project.height)
    return { boxW: width, boxH: height }
  }
  if (item.kind === 'text') {
    const { width, height } = context.measureText(item)
    return { boxW: width, boxH: height }
  }
  return null
}

export interface LayerOptions {
  connection?: TransitionConnection
  /** Static geometry only - ignores animations, effects and transitions (used for selection handles). */
  staticOnly?: boolean
}

export function buildLayer(project: Project, item: TimelineItem, trackIndex: number, time: number, context: SceneContext, options: LayerOptions = {}): Layer | null {
  const box = itemBox(project, item, context)
  if (!box) return null
  const elapsed = time - item.start
  const transform = resolveTransform(item, elapsed)
  const motion = options.staticOnly ? null : resolveMotion(item, elapsed)
  const effect = options.staticOnly || item.kind === 'shape' ? null : resolveItemEffect(item, elapsed)
  const transition = options.staticOnly ? IDLE_TRANSITION : resolveTransition(options.connection, item.id, time)

  let matrix: ColorMatrix = IDENTITY
  if (item.kind === 'video' || item.kind === 'image') {
    matrix = adjustMatrix(item.adjust)
  }
  if (effect) matrix = multiply(effect.matrix, matrix)
  matrix = multiply(transition.matrix, matrix)

  const asset = item.assetId ? context.assets.get(item.assetId) : undefined
  return {
    item,
    trackIndex,
    boxW: box.boxW,
    boxH: box.boxH,
    source: box.sx === undefined ? undefined : { sx: box.sx, sy: box.sy, sw: box.sw, sh: box.sh },
    mediaTime: item.kind === 'video' ? mediaTimeAt(item, time, project.fps, asset?.duration) : undefined,
    cx: (transform.x + (motion?.dx ?? 0) + (effect?.dx ?? 0) + transition.dx) / 100 * project.width,
    cy: (transform.y + (motion?.dy ?? 0) + (effect?.dy ?? 0) + transition.dy) / 100 * project.height,
    scale: transform.scale * (motion?.scale ?? 1) * (effect?.scale ?? 1) * transition.scale,
    rotation: transform.rotation + (motion?.rotation ?? 0) + transition.rotation,
    flipX: Boolean(item.flipX),
    flipY: Boolean(item.flipY),
    opacity: clamp(transform.opacity * (motion?.opacity ?? 1) * transition.opacity, 0, 1),
    blur: Math.max(motion?.blur ?? 0, effect?.blur ?? 0, transition.blur),
    blend: item.blend ?? 'normal',
    radius: item.kind === 'shape' ? 0 : Math.max(0, item.radius ?? 0),
    matrix: isIdentity(matrix) ? null : matrix,
    vignette: item.kind === 'video' || item.kind === 'image' ? clamp(item.adjust?.vignette ?? 0, 0, 1) : 0,
    clipLeft: transition.clipLeft,
    clipRight: Math.min(transition.clipRight, motion?.reveal ?? 1),
    grain: effect?.grain ?? 0,
    scanlines: effect?.scanlines ?? false,
    glitch: effect?.glitch ?? []
  }
}

export function itemActiveAt(item: TimelineItem, time: number) {
  return !item.hidden && time >= item.start && time < item.start + item.duration
}

/** Visual layers at `time`, ordered bottom (first) to top (last). Track 0 is the top track. */
export function resolveScene(project: Project, time: number, context: SceneContext): Layer[] {
  const layers: Layer[] = []
  for (let trackIndex = project.tracks.length - 1; trackIndex >= 0; trackIndex--) {
    const track = project.tracks[trackIndex]
    if (track.hidden || track.kind === 'audio') continue
    const connections = track.kind === 'video' ? trackConnections(track, project.fps) : []
    const active = connections.find(connection => time >= connection.start && time <= connection.end)
    const items: Array<{ item: TimelineItem; connection?: TransitionConnection }> = []
    if (active) {
      items.push({ item: active.from, connection: active }, { item: active.to, connection: active })
    }
    for (const item of track.items) {
      if (!isVisualKind(item.kind) || !itemActiveAt(item, time)) continue
      if (active && (item.id === active.from.id || item.id === active.to.id)) continue
      items.push({ item })
    }
    items.sort((left, right) => {
      if (left.connection && right.connection) return left.item.id === left.connection.from.id ? -1 : 1
      return left.item.start - right.item.start
    })
    for (const entry of items) {
      const layer = buildLayer(project, entry.item, trackIndex, time, context, { connection: entry.connection })
      if (layer && layer.opacity > 0.001) layers.push(layer)
    }
  }
  return layers
}

/** Corners of a layer box in project pixels (clockwise from top-left). */
export function layerQuad(layer: Pick<Layer, 'cx' | 'cy' | 'boxW' | 'boxH' | 'scale' | 'rotation' | 'flipX' | 'flipY'>) {
  const angle = (layer.rotation * Math.PI) / 180
  const cos = Math.cos(angle)
  const sin = Math.sin(angle)
  const hw = (layer.boxW * layer.scale) / 2
  const hh = (layer.boxH * layer.scale) / 2
  return ([[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]] as const).map(([x, y]) => ({
    x: layer.cx + x * cos - y * sin,
    y: layer.cy + x * sin + y * cos
  }))
}

export interface AudioVoice {
  item: TimelineItem
  track: Track
  gain: number
  mediaTime: number
}

export function fadeGain(item: TimelineItem, time: number) {
  const elapsed = time - item.start
  let gain = 1
  if (item.fadeIn && item.fadeIn > 0) gain *= clamp(elapsed / item.fadeIn, 0, 1)
  if (item.fadeOut && item.fadeOut > 0) gain *= clamp((item.duration - elapsed) / item.fadeOut, 0, 1)
  return gain
}

export function itemHasAudio(item: TimelineItem, assets: Map<string, MediaAsset>) {
  if (item.kind === 'audio') return true
  if (item.kind !== 'video' || !item.assetId) return false
  return assets.get(item.assetId)?.hasAudio !== false
}

/** Last moment that contributes visible pixels or audible sound. */
export function contentEnd(project: Pick<Project, 'tracks'>) {
  let end = 0
  for (const track of project.tracks) {
    if (track.hidden && track.kind !== 'audio') continue
    for (const item of track.items) {
      if (item.hidden) continue
      if (item.kind === 'audio' && (track.muted || item.muted || (item.volume ?? 1) <= 0)) continue
      end = Math.max(end, item.start + item.duration)
    }
  }
  return Math.round(end * 1000) / 1000
}
