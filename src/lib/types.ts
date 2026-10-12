/**
 * Project document model. The Rust exporter (src-tauri/src/model.rs) mirrors
 * these types, so any field added here must be added there as well.
 */

export type TrackKind = 'video' | 'text' | 'audio'
export type ItemKind = 'video' | 'image' | 'audio' | 'text' | 'shape'
export type MediaKind = 'video' | 'image' | 'audio'

export type Fit = 'contain' | 'cover' | 'fill'
export type Easing = 'linear' | 'ease-in' | 'ease-out' | 'ease-in-out'
export type BlendMode = 'normal' | 'multiply' | 'screen' | 'overlay' | 'darken' | 'lighten' | 'color-dodge' | 'color-burn' | 'hard-light' | 'soft-light' | 'difference' | 'exclusion'

export type AnimationInKind = 'none' | 'fade' | 'pop' | 'rise' | 'drop' | 'slide-left' | 'slide-right' | 'zoom-in' | 'zoom-out' | 'bounce' | 'spin' | 'blur' | 'typewriter'
export type AnimationOutKind = 'none' | 'fade' | 'shrink' | 'grow' | 'sink' | 'lift' | 'slide-left' | 'slide-right' | 'spin' | 'blur'
export type EffectKind = 'none' | 'punch' | 'shake' | 'blur' | 'vhs' | 'glitch' | 'zoom-in' | 'zoom-out' | 'flash'
export type TransitionKind = 'none' | 'fade' | 'black' | 'white' | 'slide-left' | 'slide-right' | 'slide-up' | 'slide-down' | 'zoom' | 'blur' | 'wipe-left' | 'wipe-right' | 'flash' | 'spin'
export type ShapeKind = 'rect' | 'circle' | 'line' | 'blur'

export interface Transform {
  /** Centre of the layer in % of canvas width. */
  x: number
  /** Centre of the layer in % of canvas height. */
  y: number
  scale: number
  /** Degrees, clockwise. */
  rotation: number
  opacity: number
}

export interface Keyframe {
  id: string
  /** Seconds from the clip start. */
  time: number
  transform: Transform
  /** Easing of the segment that ends at this keyframe. */
  easing: Easing
}

export interface Animation<K extends string> {
  kind: K
  duration: number
}

export interface Adjust {
  exposure: number
  brightness: number
  contrast: number
  saturation: number
  temperature: number
  tint: number
  hue: number
  vignette: number
}

export interface Crop {
  /** Normalised source rectangle (0..1). */
  x: number
  y: number
  w: number
  h: number
}

export interface TextStyle {
  content: string
  font: string
  size: number
  weight: number
  italic: boolean
  color: string
  align: 'left' | 'center' | 'right'
  letterSpacing: number
  lineHeight: number
  /** Maximum line width in % of canvas width. */
  maxWidth: number
  uppercase: boolean
  strokeColor: string
  strokeWidth: number
  backgroundColor: string
  backgroundOpacity: number
  backgroundPadding: number
  backgroundRadius: number
  shadowColor: string
  shadowOpacity: number
  shadowBlur: number
  shadowDistance: number
  /** How the highlighted word stands out: its own color, or a box behind it. */
  highlightMode: HighlightMode
  highlightColor: string
  /** Text color of the highlighted word in box mode ('' keeps the normal color). */
  highlightTextColor: string
  /** Index of the highlighted word (whitespace separated); -1 for none. */
  highlightIndex: number
}

export type HighlightMode = 'none' | 'color' | 'box'

export interface ShapeStyle {
  kind: ShapeKind
  /** % of canvas width. */
  width: number
  /** % of canvas height. */
  height: number
  color: string
  /** Blur strength for blur regions, in project pixels. */
  blur: number
}

export interface EffectRange {
  start: number
  duration: number
}

export interface TimelineItem {
  id: string
  trackId: string
  kind: ItemKind
  name: string
  start: number
  duration: number
  locked?: boolean
  hidden?: boolean
  groupId?: string

  /** Media items reference project.assets by id. */
  assetId?: string
  /** Seconds into the source where the clip begins. */
  sourceIn?: number
  speed?: number
  volume?: number
  muted?: boolean
  fadeIn?: number
  fadeOut?: number

  transform?: Transform
  keyframes?: Keyframe[]
  fit?: Fit
  /** Object-position (0..100) used when the fit crops the source. */
  focus?: { x: number; y: number }
  crop?: Crop
  flipX?: boolean
  flipY?: boolean
  radius?: number
  blend?: BlendMode
  adjust?: Adjust
  effect?: EffectKind
  /** Part of the clip the effect covers, relative to the clip start. Whole clip when absent. */
  effectRange?: EffectRange
  animIn?: Animation<AnimationInKind>
  animOut?: Animation<AnimationOutKind>
  /** Transition into this clip from the clip that ends exactly where it starts. */
  transition?: Animation<TransitionKind>

  text?: TextStyle
  shape?: ShapeStyle
}

export interface Track {
  id: string
  name: string
  kind: TrackKind
  locked?: boolean
  muted?: boolean
  hidden?: boolean
  items: TimelineItem[]
}

export interface MediaAsset {
  id: string
  kind: MediaKind
  name: string
  /** Absolute path of the original file. */
  path: string
  /** Browser-playable proxy, when the original cannot be played by the webview. */
  previewPath?: string
  /** The original cannot be played by the webview; previewPath must be generated. */
  needsProxy?: boolean
  duration?: number
  width?: number
  height?: number
  fps?: number
  hasAudio?: boolean
  size: number
  addedAt: string
}

export type ExportFormat = 'mp4' | 'mov' | 'webm'
export type ExportCodec = 'h264' | 'h265' | 'vp9'
export type ExportQuality = 'draft' | 'standard' | 'high' | 'ultra'
export type Fps = 24 | 25 | 30 | 50 | 60

export interface ExportSettings {
  format: ExportFormat
  codec: ExportCodec
  quality: ExportQuality
  /** Long edge of the rendered file in pixels (720, 1080, 1440, 2160). */
  resolution: number
  fps: Fps
  hardware: boolean
}

export interface Project {
  version: 2
  id: string
  name: string
  createdAt: string
  updatedAt: string
  width: number
  height: number
  fps: Fps
  background: string
  tracks: Track[]
  assets: MediaAsset[]
  export: ExportSettings
  /** Free-form origin info, e.g. which generator produced the draft. */
  origin?: string
}

export interface ProjectSummary {
  id: string
  name: string
  updatedAt: string
  createdAt: string
  width: number
  height: number
  duration: number
  thumbnail?: string
}

export interface CaptionCue {
  text: string
  start: number
  duration: number
  /** Word lit up while this cue shows (word-by-word highlight captions). */
  highlightIndex?: number
  /** Same caption as the previous cue, only the highlight moved on. */
  continued?: boolean
}
