import { DEFAULT_TRANSFORM, normalizeTransform } from './engine/keyframes'
import { clamp, uid } from './engine/math'
import { DEFAULT_TEXT_STYLE } from './engine/text'
import { CAPTION_PRESET, createProject, TRACK_LABELS } from './project'
import type { CaptionCue, Fps, Keyframe, MediaAsset, Project, TimelineItem, Track, TrackKind, Transform } from './types'

function track(kind: TrackKind, name: string, items: TimelineItem[]): Track {
  const id = uid('track')
  return { id, kind, name, items: items.map(item => ({ ...item, trackId: id })) }
}

function captionItems(cues: CaptionCue[], accent: string, offset = 0): TimelineItem[] {
  return cues.map((cue, index) => ({
    id: uid('caption'),
    trackId: '',
    kind: 'text',
    name: cue.text.slice(0, 40),
    start: offset + cue.start,
    duration: Math.max(0.1, cue.duration),
    groupId: 'captions-generated',
    transform: normalizeTransform({ x: 50, y: 72 }),
    text: { ...DEFAULT_TEXT_STYLE, ...CAPTION_PRESET, content: cue.text, color: index % 6 === 0 ? accent : '#ffffff' },
    animIn: { kind: 'pop', duration: 0.18 }
  }))
}

/** Deterministic fraction in [0, 1) from a string (FNV-1a). */
function stableFraction(text: string) {
  let hash = 0x811c9dc5
  for (let index = 0; index < text.length; index++) {
    hash ^= text.charCodeAt(index)
    hash = Math.imul(hash, 0x01000193)
  }
  return ((hash >>> 0) % 10000) / 10000
}

/** Picks a stable, interesting section of a long background video. */
export function backgroundOffset(background: MediaAsset, voice: MediaAsset, duration: number) {
  const length = background.duration ?? 0
  if (length <= duration) return 0
  const maxStart = length - duration
  const candidate = length * 0.25 + stableFraction(background.path + voice.path) * length * 0.4
  return clamp(candidate, 0, maxStart)
}

/**
 * Photo motion as editable keyframes: slide in, slow drift, slide out -
 * the same treatment the previous generator produced.
 */
export function editorialMotion(duration: number, variant: number, base: Transform): Keyframe[] {
  const directions = [{ x: 1.1, y: -0.45 }, { x: -1.1, y: 0.4 }, { x: 0.65, y: 0.8 }, { x: -0.7, y: -0.75 }]
  const direction = directions[((variant % 4) + 4) % 4]
  const side = variant % 2 === 0 ? 1 : -1
  const slide = Math.min(0.38, duration * 0.24)
  const start = { x: base.x - direction.x, y: base.y - direction.y }
  const end = { x: base.x + direction.x, y: base.y + direction.y }
  const endScale = base.scale * 1.065
  const frame = (time: number, x: number, y: number, scale: number, opacity: number, easing: Keyframe['easing']): Keyframe => ({
    id: uid('kf'), time, easing, transform: normalizeTransform({ x, y, scale, rotation: base.rotation, opacity })
  })
  return [
    frame(0, start.x + side * 26, start.y, base.scale, 0, 'linear'),
    frame(slide, start.x, start.y, base.scale, 1, 'ease-out'),
    frame(Math.max(slide + 0.05, duration - slide), end.x, end.y, endScale, 1, 'ease-in-out'),
    frame(duration, end.x - side * 26, end.y, endScale, 0, 'ease-in')
  ]
}

export interface ImageEvent { imageIndex: number; start: number; end: number }

/** Keeps AI suggestions inside sane bounds, or spreads images evenly without AI. */
export function normalizeImageEvents(events: ImageEvent[], imageCount: number, duration: number): ImageEvent[] {
  if (imageCount <= 0 || duration <= 1.4) return []
  const intro = Math.min(1.1, duration * 0.12)
  const maxEvents = Math.max(1, Math.min(imageCount, Math.floor((duration - intro) / 3.2)))
  const used = new Set<number>()
  const result: ImageEvent[] = []
  let lastEnd = intro - 0.25
  for (const event of [...events].sort((a, b) => a.start - b.start)) {
    if (result.length >= maxEvents || event.imageIndex < 0 || event.imageIndex >= imageCount || used.has(event.imageIndex)) continue
    const start = Math.max(intro, event.start, lastEnd + 0.25)
    let end = Math.min(duration, event.end)
    if (end - start < 1.25) end = Math.min(duration, start + 3.4)
    end = Math.min(end, start + 4.8)
    if (end - start < 0.8) continue
    result.push({ imageIndex: event.imageIndex, start, end })
    used.add(event.imageIndex)
    lastEnd = end
  }
  if (result.length) return result
  const window = (duration - intro) / maxEvents
  return Array.from({ length: maxEvents }, (_, index) => {
    const start = intro + index * window
    return { imageIndex: index, start, end: Math.min(duration, start + Math.min(4, Math.max(1.2, window - 0.25))) }
  })
}

export interface AutoEditInput {
  voice: MediaAsset
  background: MediaAsset
  images: MediaAsset[]
  events: ImageEvent[]
  cues: CaptionCue[]
  whooshIn?: MediaAsset
  whooshOut?: MediaAsset
  accent: string
  fps: Fps
  width: number
  height: number
}

export function buildAutoEditProject(input: AutoEditInput): Project {
  const duration = Math.min(input.voice.duration ?? 30, 600)
  const project = createProject({ name: input.voice.name.replace(/\.[^.]+$/, ''), width: input.width, height: input.height, fps: input.fps })
  project.origin = 'Auto-edit'
  project.assets = [input.voice, input.background, ...input.images, ...[input.whooshIn, input.whooshOut].filter((asset): asset is MediaAsset => Boolean(asset))]
  const imageItems: TimelineItem[] = []
  const sfxItems: TimelineItem[] = []
  normalizeImageEvents(input.events, input.images.length, duration).forEach((event, index) => {
    const image = input.images[event.imageIndex]
    const length = event.end - event.start
    const base = { ...DEFAULT_TRANSFORM, y: 46, scale: 0.86 }
    imageItems.push({
      id: uid('image'), trackId: '', kind: 'image', name: image.name, assetId: image.id, start: event.start, duration: length,
      fit: 'contain', radius: 28, transform: base, keyframes: editorialMotion(length, index, base)
    })
    if (input.whooshIn) sfxItems.push({ id: uid('sfx'), trackId: '', kind: 'audio', name: 'Whoosh in', assetId: input.whooshIn.id, start: Math.max(0, event.start - 0.05), duration: input.whooshIn.duration ?? 0.4, volume: 0.8, sourceIn: 0 })
    if (input.whooshOut && length > 1.2) sfxItems.push({ id: uid('sfx'), trackId: '', kind: 'audio', name: 'Whoosh out', assetId: input.whooshOut.id, start: Math.max(0, event.end - (input.whooshOut.duration ?? 0.4)), duration: input.whooshOut.duration ?? 0.4, volume: 0.8, sourceIn: 0 })
  })
  const backgroundLength = input.background.duration ?? duration
  const offset = backgroundOffset(input.background, input.voice, duration)
  const backgrounds: TimelineItem[] = []
  for (let cursor = 0, index = 0; cursor < duration - 0.05 && index < 50; index++) {
    const sourceIn = index === 0 ? offset : 0
    const length = Math.min(duration - cursor, backgroundLength - sourceIn)
    if (length <= 0.05) break
    backgrounds.push({ id: uid('video'), trackId: '', kind: 'video', name: input.background.name, assetId: input.background.id, start: cursor, duration: length, sourceIn, speed: 1, volume: 0, muted: true, fit: 'cover', transform: { ...DEFAULT_TRANSFORM } })
    cursor += length
  }
  project.tracks = [
    track('text', 'Captions', captionItems(input.cues, input.accent)),
    ...(imageItems.length ? [track('video', 'Photos', imageItems)] : []),
    track('video', TRACK_LABELS.video, backgrounds),
    track('audio', 'Voiceover', [{ id: uid('audio'), trackId: '', kind: 'audio', name: input.voice.name, assetId: input.voice.id, start: 0, duration, sourceIn: 0, speed: 1, volume: 1 }]),
    ...(sfxItems.length ? [track('audio', 'Sound effects', sfxItems)] : [])
  ]
  return project
}

export interface DraftInput {
  title: string
  hook: string
  cta: string
  cues: Array<{ text: string; start: number; duration: number; emphasis?: boolean }>
  duration: number
  accent: string
  background?: MediaAsset
  width: number
  height: number
  fps: Fps
}

export function buildDraftProject(input: DraftInput): Project {
  const project = createProject({ name: input.title || 'AI draft', width: input.width, height: input.height, fps: input.fps })
  project.origin = 'AI draft'
  const titleItems: TimelineItem[] = []
  if (input.hook) {
    titleItems.push({
      id: uid('text'), trackId: '', kind: 'text', name: 'Hook', start: 0, duration: Math.min(3.5, input.duration),
      transform: normalizeTransform({ x: 50, y: 22 }),
      text: { ...DEFAULT_TEXT_STYLE, content: input.hook.toLocaleUpperCase(), font: 'Anton', weight: 400, size: 104, strokeColor: '#000000', strokeWidth: 7, maxWidth: 84 },
      animIn: { kind: 'bounce', duration: 0.45 }, effect: 'punch'
    })
  }
  if (input.cta) {
    titleItems.push({
      id: uid('text'), trackId: '', kind: 'text', name: 'Call to action', start: Math.max(0, input.duration - 3), duration: Math.min(3, input.duration),
      transform: normalizeTransform({ x: 50, y: 82 }),
      text: { ...DEFAULT_TEXT_STYLE, content: input.cta, font: 'DM Sans', weight: 700, size: 60, color: '#000000', backgroundColor: input.accent, backgroundOpacity: 1, backgroundPadding: 20, backgroundRadius: 16, maxWidth: 80 },
      animIn: { kind: 'rise', duration: 0.4 }
    })
  }
  const cues = input.cues.map(cue => ({ text: cue.text, start: cue.start, duration: cue.duration }))
  const tracks: Track[] = [track('text', 'Titles', titleItems), track('text', 'Captions', captionItems(cues, input.accent))]
  if (input.background) {
    project.assets.push(input.background)
    const length = input.background.kind === 'image' ? input.duration : Math.min(input.duration, input.background.duration ?? input.duration)
    tracks.push(track('video', TRACK_LABELS.video, [{
      id: uid('video'), trackId: '', kind: input.background.kind === 'image' ? 'image' : 'video', name: input.background.name, assetId: input.background.id,
      start: 0, duration: length, sourceIn: 0, speed: 1, volume: 0, muted: true, fit: 'cover', transform: { ...DEFAULT_TRANSFORM },
      effect: input.background.kind === 'image' ? 'zoom-in' : undefined
    }]))
  }
  project.tracks = tracks.filter(entry => entry.items.length)
  return project
}
