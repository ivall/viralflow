import type { ExportSettings, Fps, ItemKind, MediaAsset, Project, ShapeKind, TextStyle, TimelineItem, Track, TrackKind } from './types'
import { DEFAULT_TRANSFORM } from './engine/keyframes'
import { uid } from './engine/math'
import { DEFAULT_TEXT_STYLE } from './engine/text'

export const TRACK_COLORS: Record<ItemKind, string> = {
  video: '#4f7cff',
  image: '#8b6cf6',
  audio: '#2fb38a',
  text: '#e2a03f',
  shape: '#d16ba5'
}

export const TRACK_LABELS: Record<TrackKind, string> = { video: 'Video', text: 'Text', audio: 'Audio' }

export function trackKindFor(kind: ItemKind): TrackKind {
  if (kind === 'audio') return 'audio'
  if (kind === 'text') return 'text'
  return 'video'
}

export function isVisualItem(item: Pick<TimelineItem, 'kind'>) {
  return item.kind !== 'audio'
}

export function isMediaItem(item: Pick<TimelineItem, 'kind'>) {
  return item.kind === 'video' || item.kind === 'image' || item.kind === 'audio'
}

export function hasSound(item: Pick<TimelineItem, 'kind'>) {
  return item.kind === 'video' || item.kind === 'audio'
}

export const ASPECT_PRESETS: Array<{ id: string; label: string; width: number; height: number; hint: string }> = [
  { id: '9:16', label: '9:16', width: 1080, height: 1920, hint: 'TikTok, Reels, Shorts' },
  { id: '16:9', label: '16:9', width: 1920, height: 1080, hint: 'YouTube' },
  { id: '1:1', label: '1:1', width: 1080, height: 1080, hint: 'Square post' },
  { id: '4:5', label: '4:5', width: 1080, height: 1350, hint: 'Instagram post' },
  { id: '3:4', label: '3:4', width: 1080, height: 1440, hint: 'Classic portrait' },
  { id: '21:9', label: '21:9', width: 2520, height: 1080, hint: 'Cinematic' }
]

export function aspectLabel(width: number, height: number) {
  const preset = ASPECT_PRESETS.find(candidate => Math.abs(candidate.width / candidate.height - width / height) < 0.01)
  return preset?.label ?? `${width}×${height}`
}

export const DEFAULT_EXPORT: ExportSettings = { format: 'mp4', codec: 'h264', quality: 'high', resolution: 1080, fps: 30, hardware: false }

export function createProject(options: { name?: string; width?: number; height?: number; fps?: Fps } = {}): Project {
  const now = new Date().toISOString()
  const fps = options.fps ?? 30
  return {
    version: 2,
    id: uid('project'),
    name: options.name?.trim() || 'Untitled project',
    createdAt: now,
    updatedAt: now,
    width: options.width ?? 1080,
    height: options.height ?? 1920,
    fps,
    background: '#000000',
    tracks: [],
    assets: [],
    export: { ...DEFAULT_EXPORT, fps, resolution: Math.min(options.width ?? 1080, options.height ?? 1920) }
  }
}

export function createTrack(kind: TrackKind, existing: Track[]): Track {
  const count = existing.filter(track => track.kind === kind).length + 1
  return { id: uid('track'), kind, name: count > 1 ? `${TRACK_LABELS[kind]} ${count}` : TRACK_LABELS[kind], items: [] }
}

export function mediaItemDefaults(asset: MediaAsset, project: Pick<Project, 'width' | 'height'>): Partial<TimelineItem> & { kind: ItemKind } {
  if (asset.kind === 'audio') {
    return { kind: 'audio', name: asset.name, assetId: asset.id, duration: asset.duration || 5, sourceIn: 0, speed: 1, volume: 1 }
  }
  const landscapeOnPortrait = (asset.width ?? 1) / (asset.height ?? 1) > project.width / project.height + 0.01
  return {
    kind: asset.kind,
    name: asset.name,
    assetId: asset.id,
    duration: asset.kind === 'image' ? 4 : asset.duration || 5,
    sourceIn: 0,
    speed: 1,
    volume: 1,
    fit: asset.kind === 'video' && !landscapeOnPortrait ? 'cover' : 'contain',
    transform: { ...DEFAULT_TRANSFORM }
  }
}

export interface TextPreset {
  id: string
  label: string
  style: Partial<TextStyle>
  animIn?: TimelineItem['animIn']
}

export const TEXT_PRESETS: TextPreset[] = [
  { id: 'plain', label: 'Plain', style: { font: 'Inter', weight: 700, size: 72 } },
  { id: 'bold', label: 'Bold', style: { font: 'Manrope', weight: 800, size: 88, strokeColor: '#000000', strokeWidth: 6 }, animIn: { kind: 'pop', duration: 0.3 } },
  { id: 'impact', label: 'Yellow impact', style: { font: 'Anton', weight: 400, size: 96, color: '#ffd43b', strokeColor: '#111111', strokeWidth: 6, uppercase: true }, animIn: { kind: 'bounce', duration: 0.45 } },
  { id: 'caption', label: 'Boxed caption', style: { font: 'DM Sans', weight: 700, size: 64, backgroundColor: '#000000', backgroundOpacity: 0.75, backgroundPadding: 18, backgroundRadius: 14 }, animIn: { kind: 'fade', duration: 0.3 } },
  { id: 'label', label: 'Label', style: { font: 'Bebas Neue', weight: 400, size: 92, backgroundColor: '#ef3340', backgroundOpacity: 1, backgroundPadding: 14, backgroundRadius: 6, uppercase: true }, animIn: { kind: 'rise', duration: 0.4 } },
  { id: 'glow', label: 'Glow', style: { font: 'Rubik', weight: 800, size: 84, color: '#e9fbff', shadowColor: '#4cc9f0', shadowOpacity: 1, shadowBlur: 28, shadowDistance: 0 }, animIn: { kind: 'blur', duration: 0.4 } },
  { id: 'shadow', label: 'Shadow', style: { font: 'Montserrat', weight: 800, size: 80, shadowColor: '#000000', shadowOpacity: 0.85, shadowBlur: 10, shadowDistance: 8 } },
  { id: 'outline', label: 'Outline', style: { font: 'Poppins', weight: 700, size: 84, color: '#ffffff', strokeColor: '#7c3aed', strokeWidth: 8 } },
  { id: 'serif', label: 'Elegant', style: { font: 'Georgia', weight: 400, size: 76, italic: true } },
  { id: 'condensed', label: 'Condensed', style: { font: 'Oswald', weight: 700, size: 90, uppercase: true, letterSpacing: 2, strokeColor: '#000000', strokeWidth: 4 } }
]

/** A preset replaces the whole look; only the text and its layout box carry over. */
export function presetTextStyle(current: TextStyle, preset: TextPreset): TextStyle {
  return { ...DEFAULT_TEXT_STYLE, content: current.content, align: current.align, maxWidth: current.maxWidth, ...preset.style }
}

export const CAPTION_PRESET: Partial<TextStyle> = { font: 'Manrope', weight: 800, size: 78, strokeColor: '#000000', strokeWidth: 6, maxWidth: 80 }

export function textItemDefaults(content = DEFAULT_TEXT_STYLE.content, preset?: TextPreset): Partial<TimelineItem> & { kind: ItemKind } {
  return {
    kind: 'text',
    name: content.slice(0, 40) || 'Text',
    duration: 3,
    transform: { ...DEFAULT_TRANSFORM, y: 50 },
    text: { ...DEFAULT_TEXT_STYLE, ...preset?.style, content },
    animIn: preset?.animIn
  }
}

export function stickerDefaults(emoji: string): Partial<TimelineItem> & { kind: ItemKind } {
  return {
    kind: 'text',
    name: `Sticker ${emoji}`,
    duration: 3,
    transform: { ...DEFAULT_TRANSFORM, y: 40 },
    text: { ...DEFAULT_TEXT_STYLE, content: emoji, size: 220, weight: 400, font: 'Segoe UI Emoji' }
  }
}

export const SHAPE_PRESETS: Array<{ kind: ShapeKind; label: string; width: number; height: number; color: string; radius: number }> = [
  { kind: 'rect', label: 'Rectangle', width: 60, height: 12, color: '#ffffff', radius: 24 },
  { kind: 'circle', label: 'Circle', width: 30, height: 30, color: '#4cc9f0', radius: 0 },
  { kind: 'line', label: 'Line', width: 60, height: 0.4, color: '#ffffff', radius: 999 },
  { kind: 'blur', label: 'Blur region', width: 60, height: 14, color: '#ffffff', radius: 28 }
]

export function shapeDefaults(kind: ShapeKind): Partial<TimelineItem> & { kind: ItemKind } {
  const preset = SHAPE_PRESETS.find(candidate => candidate.kind === kind) ?? SHAPE_PRESETS[0]
  return {
    kind: 'shape',
    name: preset.label,
    duration: 3,
    radius: preset.radius,
    transform: { ...DEFAULT_TRANSFORM },
    shape: { kind, width: preset.width, height: preset.height, color: preset.color, blur: 30 }
  }
}

export function allItems(project: Pick<Project, 'tracks'>) {
  return project.tracks.flatMap(track => track.items)
}

export function cloneProject<T>(value: T): T {
  return structuredClone(value)
}
