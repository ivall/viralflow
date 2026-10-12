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

export type TextPresetCategory = 'Highlight' | 'Captions' | 'Titles' | 'Boxes' | 'Glow'

export interface TextPreset {
  id: string
  label: string
  category: TextPresetCategory
  /** Text a new clip starts with, and what the preview card shows. */
  sample: string
  style: Partial<TextStyle>
  animIn?: TimelineItem['animIn']
}

export const TEXT_PRESET_CATEGORIES: TextPresetCategory[] = ['Highlight', 'Captions', 'Titles', 'Boxes', 'Glow']

const HEAVY_OUTLINE: Partial<TextStyle> = { font: 'Montserrat', weight: 900, size: 92, uppercase: true, strokeColor: '#000000', strokeWidth: 9, shadowColor: '#000000', shadowOpacity: 0.9, shadowBlur: 0, shadowDistance: 6 }

/** Each preset looks clearly different; font-only or case-only variants are left to the inspector. */
export const TEXT_PRESETS: TextPreset[] = [
  // Highlight: captions light up the word being spoken
  { id: 'hl-yellow', label: 'Yellow word', category: 'Highlight', sample: 'THIS CHANGED EVERYTHING', style: { ...HEAVY_OUTLINE, highlightMode: 'color', highlightColor: '#ffe14d', highlightIndex: 1 }, animIn: { kind: 'pop', duration: 0.18 } },
  { id: 'hl-green', label: 'Green word', category: 'Highlight', sample: 'you need to hear this', style: { font: 'Poppins', weight: 700, size: 76, shadowColor: '#000000', shadowOpacity: 0.85, shadowBlur: 16, shadowDistance: 2, highlightMode: 'color', highlightColor: '#4ef08a', highlightIndex: 3 }, animIn: { kind: 'pop', duration: 0.18 } },
  { id: 'hl-box-purple', label: 'Purple box', category: 'Highlight', sample: 'WATCH THIS PART', style: { font: 'Montserrat', weight: 900, size: 88, uppercase: true, shadowColor: '#000000', shadowOpacity: 0.6, shadowBlur: 12, shadowDistance: 3, highlightMode: 'box', highlightColor: '#7c3aed', highlightIndex: 1 }, animIn: { kind: 'pop', duration: 0.18 } },
  { id: 'hl-box-yellow', label: 'Yellow box', category: 'Highlight', sample: 'the secret is simple', style: { font: 'DM Sans', weight: 800, size: 78, shadowColor: '#000000', shadowOpacity: 0.6, shadowBlur: 12, shadowDistance: 3, highlightMode: 'box', highlightColor: '#ffe14d', highlightTextColor: '#111111', highlightIndex: 1 }, animIn: { kind: 'pop', duration: 0.18 } },
  { id: 'hl-pink', label: 'Pink on dark', category: 'Highlight', sample: 'and then she said', style: { font: 'DM Sans', weight: 700, size: 66, backgroundColor: '#000000', backgroundOpacity: 0.72, backgroundPadding: 18, backgroundRadius: 14, highlightMode: 'color', highlightColor: '#ff4fa3', highlightIndex: 3 }, animIn: { kind: 'fade', duration: 0.18 } },

  // Captions: readable over any footage
  { id: 'hormozi', label: 'Punchy', category: 'Captions', sample: 'NOBODY TALKS ABOUT THIS', style: { ...HEAVY_OUTLINE }, animIn: { kind: 'pop', duration: 0.25 } },
  { id: 'bold', label: 'Bold outline', category: 'Captions', sample: 'here is the thing', style: { font: 'Manrope', weight: 800, size: 80, strokeColor: '#000000', strokeWidth: 6 }, animIn: { kind: 'pop', duration: 0.25 } },
  { id: 'caption-soft', label: 'Soft shadow', category: 'Captions', sample: 'wait until the end', style: { font: 'Inter', weight: 800, size: 72, shadowColor: '#000000', shadowOpacity: 0.85, shadowBlur: 18, shadowDistance: 0 }, animIn: { kind: 'typewriter', duration: 0.6 } },
  { id: 'caption', label: 'Boxed', category: 'Captions', sample: 'let me explain', style: { font: 'DM Sans', weight: 700, size: 64, backgroundColor: '#000000', backgroundOpacity: 0.75, backgroundPadding: 18, backgroundRadius: 14 }, animIn: { kind: 'fade', duration: 0.3 } },
  { id: 'plain', label: 'Plain', category: 'Captions', sample: 'Your text', style: { font: 'Inter', weight: 700, size: 72 } },

  // Titles: hooks and headings
  { id: 'beast', label: 'Challenge', category: 'Titles', sample: '24 HOUR CHALLENGE', style: { font: 'Archivo Black', weight: 400, size: 96, uppercase: true, color: '#ffd400', strokeColor: '#000000', strokeWidth: 11, shadowColor: '#000000', shadowOpacity: 1, shadowBlur: 0, shadowDistance: 9 }, animIn: { kind: 'bounce', duration: 0.45 } },
  { id: 'pop-3d', label: 'Pop 3D', category: 'Titles', sample: 'NO WAY', style: { font: 'Rubik', weight: 900, size: 110, uppercase: true, strokeColor: '#111111', strokeWidth: 6, shadowColor: '#ff2d55', shadowOpacity: 1, shadowBlur: 0, shadowDistance: 12 }, animIn: { kind: 'drop', duration: 0.4 } },
  { id: 'sticker', label: 'Sticker', category: 'Titles', sample: 'so good', style: { font: 'Poppins', weight: 700, size: 96, color: '#111111', strokeColor: '#ffffff', strokeWidth: 12, shadowColor: '#000000', shadowOpacity: 0.35, shadowBlur: 14, shadowDistance: 6 }, animIn: { kind: 'pop', duration: 0.3 } },
  { id: 'headline', label: 'Headline', category: 'Titles', sample: '3 THINGS I WISH I KNEW', style: { font: 'Archivo Black', weight: 400, size: 88, uppercase: true, shadowColor: '#000000', shadowOpacity: 0.7, shadowBlur: 20, shadowDistance: 6 }, animIn: { kind: 'zoom-in', duration: 0.35 } },
  { id: 'outline', label: 'Color outline', category: 'Titles', sample: 'Level up', style: { font: 'Poppins', weight: 700, size: 88, color: '#ffffff', strokeColor: '#7c3aed', strokeWidth: 8 }, animIn: { kind: 'rise', duration: 0.35 } },
  { id: 'retro', label: 'Retro', category: 'Titles', sample: 'THROWBACK', style: { font: 'Barlow Condensed', weight: 700, size: 110, uppercase: true, color: '#ffcc4d', strokeColor: '#e4572e', strokeWidth: 6, shadowColor: '#2b2d42', shadowOpacity: 1, shadowBlur: 0, shadowDistance: 10 }, animIn: { kind: 'drop', duration: 0.4 } },
  { id: 'minimal', label: 'Minimal', category: 'Titles', sample: 'CHAPTER ONE', style: { font: 'Inter', weight: 500, size: 56, uppercase: true, letterSpacing: 10 }, animIn: { kind: 'fade', duration: 0.5 } },
  { id: 'serif', label: 'Elegant', category: 'Titles', sample: 'a little story', style: { font: 'Georgia', weight: 400, size: 80, italic: true }, animIn: { kind: 'blur', duration: 0.5 } },

  // Boxes: labels and tags
  { id: 'label', label: 'Red label', category: 'Boxes', sample: 'BREAKING', style: { font: 'Bebas Neue', weight: 400, size: 96, backgroundColor: '#ef3340', backgroundOpacity: 1, backgroundPadding: 14, backgroundRadius: 6, uppercase: true }, animIn: { kind: 'rise', duration: 0.4 } },
  { id: 'storytime', label: 'White box', category: 'Boxes', sample: 'storytime', style: { font: 'DM Sans', weight: 700, size: 72, color: '#111111', backgroundColor: '#ffffff', backgroundOpacity: 1, backgroundPadding: 18, backgroundRadius: 16 }, animIn: { kind: 'pop', duration: 0.3 } },
  { id: 'tag-yellow', label: 'Yellow tag', category: 'Boxes', sample: 'MUST WATCH', style: { font: 'Archivo Black', weight: 400, size: 68, color: '#111111', backgroundColor: '#ffe14d', backgroundOpacity: 1, backgroundPadding: 16, backgroundRadius: 10, uppercase: true }, animIn: { kind: 'pop', duration: 0.3 } },
  { id: 'tag-blue', label: 'Blue pill', category: 'Boxes', sample: 'Tip #1', style: { font: 'DM Sans', weight: 700, size: 66, color: '#ffffff', backgroundColor: '#2f6bff', backgroundOpacity: 1, backgroundPadding: 18, backgroundRadius: 60 }, animIn: { kind: 'slide-left', duration: 0.35 } },
  { id: 'tag-pink', label: 'Pastel', category: 'Boxes', sample: 'get ready with me', style: { font: 'Poppins', weight: 700, size: 60, color: '#3a1027', backgroundColor: '#ffd1e3', backgroundOpacity: 1, backgroundPadding: 18, backgroundRadius: 18 }, animIn: { kind: 'pop', duration: 0.3 } },

  // Glow: neon and night looks
  { id: 'glow', label: 'Ice glow', category: 'Glow', sample: 'late night', style: { font: 'Rubik', weight: 800, size: 88, color: '#e9fbff', shadowColor: '#4cc9f0', shadowOpacity: 1, shadowBlur: 28, shadowDistance: 0 }, animIn: { kind: 'blur', duration: 0.4 } },
  { id: 'neon-pink', label: 'Neon pink', category: 'Glow', sample: 'vibes only', style: { font: 'Poppins', weight: 700, size: 88, color: '#fff0fa', shadowColor: '#ff2bd6', shadowOpacity: 1, shadowBlur: 30, shadowDistance: 0 }, animIn: { kind: 'zoom-in', duration: 0.35 } },
  { id: 'gold', label: 'Gold', category: 'Glow', sample: 'WINNER', style: { font: 'Anton', weight: 400, size: 110, uppercase: true, color: '#ffd977', strokeColor: '#5a3a00', strokeWidth: 5, shadowColor: '#ffb800', shadowOpacity: 0.8, shadowBlur: 26, shadowDistance: 0 }, animIn: { kind: 'zoom-in', duration: 0.4 } }
]

/** A preset replaces the whole look; only the text and its layout box carry over. */
export function presetTextStyle(current: TextStyle, preset: TextPreset): TextStyle {
  const style = { ...DEFAULT_TEXT_STYLE, content: current.content, align: current.align, maxWidth: current.maxWidth, ...preset.style }
  // Word-highlight captions keep the word they light up.
  if (current.highlightIndex >= 0) style.highlightIndex = current.highlightIndex
  return style
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
