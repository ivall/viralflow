import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { CaptionCue, MediaKind, Project, ProjectSummary } from '@/lib/types'

/** Typed bridge to the Rust commands in src-tauri/src/commands.rs. */

export interface Settings {
  ffmpegPath: string
  openrouterApiKey: string
  openrouterModel: string
  openrouterTranscriptionModel: string
  transcriptionProvider: 'local' | 'openrouter'
  whisperModel: string
  captionLanguage: string
  defaultAspect: string
  defaultFps: number
  autosave: boolean
  snap: boolean
  safeArea: boolean
  exportFolder: string
  hardwareEncoding: boolean
}

export interface ToolsStatus {
  found: boolean
  ffmpeg?: string
  ffprobe?: string
  version?: string
  whisper: boolean
  hardwareEncoders: string[]
}

export interface ImportedMedia {
  kind: MediaKind
  name: string
  path: string
  duration?: number
  width?: number
  height?: number
  fps?: number
  hasAudio: boolean
  size: number
  needsProxy: boolean
}

export interface Filmstrip {
  path: string
  count: number
  interval: number
  tileWidth: number
  tileHeight: number
}

export interface WhisperModel {
  id: string
  label: string
  size: number
  installed: boolean
}

export interface Transcript {
  text: string
  cues: CaptionCue[]
  words: Array<{ word: string; start: number; end: number }>
  alignment: 'word' | 'segment' | 'estimated'
}

export interface ScriptPlan {
  title: string
  script: string
  hook: string
  cta: string
  textCues: Array<{ text: string; start: number; duration: number; emphasis: boolean }>
}

export interface SystemFont {
  name: string
  aliases: string[]
  weights: number[]
  italic: boolean
}

export interface OpenRouterModel {
  id: string
  name: string
  contextLength: number
  /** USD per million input tokens. */
  promptPrice: number
  /** USD per million output tokens. */
  completionPrice: number
  vision: boolean
  jsonMode: boolean
  created: number
}

export interface ExportProgress {
  jobId: string
  progress: number
  stage: string
  frame: number
  totalFrames: number
  speed: number
  etaSeconds?: number
}

export interface ExportFinished {
  jobId: string
  path?: string
  error?: string
  cancelled: boolean
}

export interface TextRaster {
  itemId: string
  png: string
  width: number
  height: number
}

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

export function errorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  return 'Something went wrong'
}

export function fileUrl(path: string | undefined) {
  if (!path) return ''
  return isTauri ? convertFileSrc(path) : path
}

export const ipc = {
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  toolsStatus: () => invoke<ToolsStatus>('tools_status'),

  listProjects: () => invoke<ProjectSummary[]>('list_projects'),
  loadProject: (id: string) => invoke<Project>('load_project', { id }),
  saveProject: (project: Project) => invoke<void>('save_project', { id: project.id, document: project }),
  deleteProject: (id: string) => invoke<void>('delete_project', { id }),
  duplicateProject: (id: string, newId: string, name: string) => invoke<Project>('duplicate_project', { id, newId, name }),
  saveProjectThumbnail: (id: string, jpegBase64: string) => invoke<string>('save_project_thumbnail', { id, jpegBase64 }),
  writeProjectFile: (path: string, document: Project) => invoke<void>('write_project_file', { path, document }),
  readProjectFile: (path: string) => invoke<Project>('read_project_file', { path }),
  writeTextFile: (path: string, contents: string) => invoke<void>('write_text_file', { path, contents }),
  readTextFile: (path: string) => invoke<string>('read_text_file', { path }),

  importMedia: (paths: string[]) => invoke<Array<{ Ok?: ImportedMedia; Err?: string }>>('import_media', { paths }),
  allowMedia: (paths: string[]) => invoke<void>('allow_media', { paths }),
  checkPaths: (paths: string[]) => invoke<boolean[]>('check_paths', { paths }),
  createProxy: (path: string, kind: MediaKind, duration?: number) => invoke<string>('create_proxy', { path, kind, duration }),
  filmstrip: (path: string, duration: number, width: number, height: number) => invoke<Filmstrip>('media_filmstrip', { path, duration, width, height }),
  waveform: async (path: string) => new Uint8Array(await invoke<ArrayBuffer>('media_waveform', { path })),
  systemFonts: () => invoke<SystemFont[]>('system_fonts'),
  builtinSounds: () => invoke<Array<{ name: string; path: string; duration: number }>>('builtin_sounds'),
  revealPath: (path: string) => invoke<void>('reveal_path', { path }),
  openPath: (path: string) => invoke<void>('open_path', { path }),

  defaultExportPath: (name: string, format: string) => invoke<string>('default_export_path', { name, format }),
  startExport: (jobId: string, request: { project: Project; outputPath: string; rasters: TextRaster[] }) => invoke<void>('start_export', { jobId, request }),
  cancelExport: (jobId: string) => invoke<void>('cancel_export', { jobId }),

  whisperModels: () => invoke<WhisperModel[]>('whisper_models'),
  downloadWhisperModel: (id: string) => invoke<void>('download_whisper_model', { id }),
  transcribe: (request: { path: string; offset: number; duration: number; language: string; options: { maxWords: number } }) => invoke<Transcript>('transcribe', { request }),

  aiPlan: (brief: { topic: string; goal: string; platform: string; language: string; duration: number; tone: string; cta: string; script: string }) => invoke<ScriptPlan>('ai_plan', { brief }),
  openrouterModels: (transcription = false) => invoke<OpenRouterModel[]>('openrouter_models', { transcription }),
  aiRewrite: (text: string, instruction: string) => invoke<string>('ai_rewrite', { text, instruction }),
  aiMatchImages: (transcript: string, cues: unknown, duration: number, images: string[]) => invoke<Array<{ imageIndex: number; start: number; end: number }>>('ai_match_images', { transcript, cues, duration, images })
}

export function onEvent<T>(name: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(name, event => handler(event.payload))
}
