import { DEFAULT_TRANSFORM, normalizeTransform } from './engine/keyframes'
import { uid } from './engine/math'
import { DEFAULT_TEXT_STYLE } from './engine/text'
import { DEFAULT_EXPORT, trackKindFor } from './project'
import type { AnimationInKind, ExportSettings, Fps, ItemKind, MediaAsset, Project, TimelineItem, Track, TrackKind, TransitionKind } from './types'

const FPS: Fps[] = [24, 25, 30, 50, 60]

function finite(value: unknown, fallback: number) {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function sanitizeExport(value: Partial<ExportSettings> | undefined, fps: Fps, width: number, height: number): ExportSettings {
  const result = { ...DEFAULT_EXPORT, fps, resolution: Math.min(width, height), ...value }
  if (!['mp4', 'mov', 'webm'].includes(result.format)) result.format = 'mp4'
  if (result.format === 'webm') result.codec = 'vp9'
  if (result.codec === 'vp9' && result.format !== 'webm') result.codec = 'h264'
  if (!FPS.includes(result.fps)) result.fps = fps
  result.resolution = Math.min(4320, Math.max(240, Math.round(finite(result.resolution, 1080))))
  return result
}

/** Brings any stored document to the current schema and repairs invalid values. */
export function migrateProject(input: unknown): Project {
  const source = (input ?? {}) as Record<string, unknown>
  if (source.version !== 2) return convertLegacyProject(source)
  const project = source as unknown as Project
  const width = Math.round(finite(project.width, 1080))
  const height = Math.round(finite(project.height, 1920))
  const fps = FPS.includes(project.fps) ? project.fps : 30
  const assets = Array.isArray(project.assets) ? project.assets.filter(asset => asset && asset.id && asset.path) : []
  const tracks: Track[] = (Array.isArray(project.tracks) ? project.tracks : []).map(track => ({
    ...track,
    id: track.id || uid('track'),
    name: track.name || 'Track',
    kind: (['video', 'text', 'audio'] as TrackKind[]).includes(track.kind) ? track.kind : 'video',
    items: [] as TimelineItem[]
  }))
  ;(Array.isArray(project.tracks) ? project.tracks : []).forEach((track, index) => {
    tracks[index].items = (Array.isArray(track.items) ? track.items : [])
      .filter(item => item && item.id && item.kind)
      .map(item => ({
        ...item,
        trackId: tracks[index].id,
        start: Math.max(0, finite(item.start, 0)),
        duration: Math.max(0.05, finite(item.duration, 3))
      }))
  })
  return {
    ...project,
    version: 2,
    id: project.id || uid('project'),
    name: project.name || 'Untitled project',
    createdAt: project.createdAt || new Date().toISOString(),
    updatedAt: project.updatedAt || new Date().toISOString(),
    width,
    height,
    fps,
    background: /^#[\da-f]{6}$/i.test(project.background ?? '') ? project.background : '#000000',
    tracks,
    assets,
    export: sanitizeExport(project.export, fps, width, height)
  }
}

// ------------------------------------------------------------------ legacy v1

type Legacy = Record<string, any>

export interface LegacyContext {
  /** Maps a v1 media URL (e.g. /api/v1/media/123-file.mp4) to a local asset, if the file exists. */
  resolveMedia?: (source: string) => MediaAsset | undefined
}

const LEGACY_ANIMATIONS: Record<string, AnimationInKind> = { pop: 'pop', rise: 'rise', slide: 'slide-left', bounce: 'bounce', fade: 'fade', karaoke: 'pop' }
const LEGACY_TRANSITIONS: Record<string, TransitionKind> = { fade: 'fade', 'slide-left': 'slide-left', 'slide-right': 'slide-right', slide: 'slide-left', zoom: 'zoom', blur: 'blur', 'wipe-left': 'wipe-left', 'wipe-right': 'wipe-right', flash: 'flash', spin: 'spin' }

function convertLegacyItem(item: Legacy, trackId: string, context: LegacyContext, assets: Map<string, MediaAsset>): TimelineItem | null {
  const kind: ItemKind = item.kind === 'overlay' ? 'image' : item.kind === 'caption' ? 'text' : item.kind
  if (!['video', 'image', 'audio', 'text', 'shape'].includes(kind)) return null
  const transform = normalizeTransform({
    x: item.position?.x ?? 50,
    y: item.position?.y ?? 50,
    scale: (item.transform?.scale ?? 1) * (kind === 'image' ? 0.72 : 1),
    rotation: item.transform?.rotation ?? 0,
    opacity: item.transform?.opacity ?? 1
  })
  const result: TimelineItem = {
    id: String(item.id || uid('item')),
    trackId,
    kind,
    name: String(item.name || kind),
    start: Math.max(0, finite(item.start, 0)),
    duration: Math.max(0.05, finite(item.duration, 3)),
    locked: Boolean(item.locked) || undefined,
    hidden: Boolean(item.hidden) || undefined,
    groupId: item.groupId || undefined
  }
  if (kind === 'video' || kind === 'image' || kind === 'audio') {
    const asset = item.source && context.resolveMedia ? context.resolveMedia(String(item.source)) : undefined
    if (!asset) return null
    assets.set(asset.id, asset)
    result.assetId = asset.id
    result.sourceIn = Math.max(0, finite(item.sourceOffset, 0))
    result.speed = finite(item.playbackRate, 1)
    result.volume = kind === 'video' ? finite(item.volume, 0) : finite(item.volume, 1)
    result.muted = Boolean(item.muted) || undefined
  }
  if (kind !== 'audio') {
    result.transform = transform
    if (Array.isArray(item.keyframes) && item.keyframes.length) {
      result.keyframes = item.keyframes.map((keyframe: Legacy) => ({
        id: String(keyframe.id || uid('kf')),
        time: finite(keyframe.time, 0),
        easing: ['linear', 'ease-in', 'ease-out', 'ease-in-out'].includes(keyframe.easing) ? keyframe.easing : 'linear',
        transform: normalizeTransform({
          x: keyframe.position?.x,
          y: keyframe.position?.y,
          scale: (keyframe.transform?.scale ?? 1) * (kind === 'image' ? 0.72 : 1),
          rotation: keyframe.transform?.rotation,
          opacity: keyframe.transform?.opacity
        })
      }))
    }
  }
  if (kind === 'video' || kind === 'image') {
    result.fit = kind === 'video' ? (item.fit ?? 'cover') : 'contain'
    if (item.crop) result.focus = { x: finite(item.crop.x, 50), y: finite(item.crop.y, 50) }
    result.radius = finite(item.borderRadius, 0) || undefined
    const transition = LEGACY_TRANSITIONS[item.transition]
    if (transition) result.transition = { kind: transition, duration: finite(item.transitionDuration, 0.4) }
    if (['punch', 'blur', 'shake', 'vhs'].includes(item.effect)) result.effect = item.effect
  }
  const animation = LEGACY_ANIMATIONS[item.animation]
  if (animation) result.animIn = { kind: animation, duration: finite(item.animationDuration, 0.3) }
  if (kind === 'text') {
    result.text = {
      ...DEFAULT_TEXT_STYLE,
      content: String(item.text ?? ''),
      font: item.fontFamily || 'Manrope',
      size: finite(item.fontSize, 58),
      weight: finite(item.fontWeight, 800),
      color: item.color || '#ffffff',
      align: item.align || 'center',
      strokeColor: item.textStrokeColor || '#101010',
      strokeWidth: finite(item.textStrokeWidth, 4.2),
      backgroundColor: item.textBackgroundColor || '#000000',
      backgroundOpacity: finite(item.textBackgroundOpacity, 0),
      backgroundPadding: finite(item.textBackgroundPadding, 12),
      lineHeight: 1.02,
      letterSpacing: -0.045 * finite(item.fontSize, 58)
    }
    result.name = result.text.content.slice(0, 40) || 'Text'
  }
  if (kind === 'shape') {
    const graphic = item.graphic === 'square' || item.graphic === 'rectangle' ? 'rect' : item.graphic
    result.shape = { kind: (['rect', 'circle', 'line', 'blur'].includes(graphic) ? graphic : 'rect'), width: finite(item.size?.width, 32), height: finite(item.size?.height, 24), color: item.color || '#ffffff', blur: finite(item.blurAmount, 22) }
    result.radius = finite(item.borderRadius, 0)
  }
  return result
}

export function convertLegacyProject(source: Legacy, context: LegacyContext = {}): Project {
  const exportSettings = source.export ?? {}
  const width = Math.round(finite(exportSettings.width, 1080))
  const height = Math.round(finite(exportSettings.height, 1920))
  const fps: Fps = FPS.includes(exportSettings.fps) ? exportSettings.fps : 30
  const assets = new Map<string, MediaAsset>()
  const tracks: Track[] = (Array.isArray(source.tracks) ? source.tracks : []).map((track: Legacy) => {
    const kind: TrackKind = track.kind === 'audio' ? 'audio' : track.kind === 'text' || track.kind === 'caption' ? 'text' : 'video'
    const id = String(track.id || uid('track'))
    const items = (Array.isArray(track.items) ? track.items : [])
      .map((item: Legacy) => convertLegacyItem(item, id, context, assets))
      .filter((item: TimelineItem | null): item is TimelineItem => Boolean(item) && trackKindFor(item!.kind) === kind)
    return { id, name: String(track.name || 'Track'), kind, locked: Boolean(track.locked) || undefined, muted: Boolean(track.muted) || undefined, hidden: Boolean(track.hidden) || undefined, items }
  })
  return migrateProject({
    version: 2,
    id: String(source.id || uid('project')),
    name: String(source.name || 'Imported project'),
    createdAt: source.createdAt || new Date().toISOString(),
    updatedAt: source.updatedAt || new Date().toISOString(),
    width,
    height,
    fps,
    background: '#11120f',
    tracks,
    assets: [...assets.values()],
    export: { ...DEFAULT_EXPORT, fps, resolution: Math.min(width, height), format: exportSettings.format === 'webm' ? 'webm' : exportSettings.format === 'mov' ? 'mov' : 'mp4' },
    origin: 'ViralFlow 1.x'
  })
}

export { DEFAULT_TRANSFORM }
