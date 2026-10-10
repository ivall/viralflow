import { open } from '@tauri-apps/plugin-dialog'
import { reactive } from 'vue'
import { uid } from '@/lib/engine/math'
import type { MediaAsset } from '@/lib/types'
import { useEditor } from '@/stores/editor'
import { useToasts } from '@/stores/toast'
import { errorMessage, ipc, onEvent, type Filmstrip } from './ipc'

export const MEDIA_EXTENSIONS = ['mp4', 'mov', 'm4v', 'webm', 'mkv', 'avi', 'wmv', 'flv', 'mts', 'm2ts', 'ts', '3gp', 'gif', 'mp3', 'wav', 'm4a', 'aac', 'ogg', 'oga', 'opus', 'flac', 'aiff', 'aif', 'wma', 'png', 'jpg', 'jpeg', 'webp', 'bmp', 'tif', 'tiff', 'avif', 'heic']

export async function pickMediaFiles(): Promise<string[]> {
  const selection = await open({ multiple: true, title: 'Import media', filters: [{ name: 'Video, audio and images', extensions: MEDIA_EXTENSIONS }] })
  if (!selection) return []
  return Array.isArray(selection) ? selection : [selection]
}

/** Progress of preview proxies, keyed by source path (0..1). */
export const proxyProgress = reactive(new Map<string, number>())
let proxyListener: Promise<unknown> | null = null

function listenToProxies() {
  proxyListener ??= onEvent<{ path: string; progress: number }>('proxy-progress', ({ path, progress }) => proxyProgress.set(path, progress))
}

export interface ProbedMedia {
  assets: MediaAsset[]
  needsProxy: MediaAsset[]
  failures: string[]
}

/** Reads metadata of local files and turns them into assets (no project needed). */
export async function probeMedia(paths: string[], known: Map<string, MediaAsset> = new Map()): Promise<ProbedMedia> {
  const result: ProbedMedia = { assets: [], needsProxy: [], failures: [] }
  if (!paths.length) return result
  for (const entry of await ipc.importMedia(paths)) {
    if (entry.Err || !entry.Ok) {
      result.failures.push(entry.Err ?? 'Unknown error')
      continue
    }
    const media = entry.Ok
    const existing = known.get(media.path)
    if (existing) {
      result.assets.push(existing)
      continue
    }
    const asset: MediaAsset = {
      id: uid('asset'),
      kind: media.kind,
      name: media.name,
      path: media.path,
      duration: media.duration,
      width: media.width,
      height: media.height,
      fps: media.fps,
      hasAudio: media.hasAudio,
      size: media.size,
      addedAt: new Date().toISOString(),
      needsProxy: media.needsProxy || undefined
    }
    result.assets.push(asset)
    if (media.needsProxy) result.needsProxy.push(asset)
  }
  return result
}

/** Probes files and adds them to the open project. Unplayable files get a preview proxy in the background. */
export async function importIntoProject(paths: string[]): Promise<MediaAsset[]> {
  const editor = useEditor()
  const toasts = useToasts()
  if (!paths.length || !editor.project) return []
  let probed: ProbedMedia
  try {
    probed = await probeMedia(paths, new Map(editor.project.assets.map(asset => [asset.path, asset])))
  } catch (error) {
    toasts.error('Import failed', errorMessage(error))
    return []
  }
  editor.registerAssets(probed.assets)
  if (probed.failures.length) toasts.error(probed.failures.length === 1 ? 'A file could not be imported' : `${probed.failures.length} files could not be imported`, probed.failures.slice(0, 2).join('\n'))
  for (const asset of probed.needsProxy) void createPreview(asset)
  return probed.assets
}

export async function createPreview(asset: MediaAsset) {
  listenToProxies()
  const editor = useEditor()
  proxyProgress.set(asset.path, 0)
  try {
    const previewPath = await ipc.createProxy(asset.path, asset.kind, asset.duration)
    editor.updateAsset(asset.id, { previewPath })
  } catch (error) {
    useToasts().error(`Preview of ${asset.name} could not be prepared`, errorMessage(error))
  } finally {
    proxyProgress.delete(asset.path)
  }
}

/** Lets the user point a missing asset to a new file. */
export async function relinkAsset(asset: MediaAsset) {
  const editor = useEditor()
  const selection = await open({ multiple: false, title: `Locate ${asset.name}`, filters: [{ name: 'Media', extensions: MEDIA_EXTENSIONS }] })
  if (!selection || Array.isArray(selection)) return
  const [result] = await ipc.importMedia([selection])
  if (!result?.Ok) {
    useToasts().error('This file cannot be used', result?.Err)
    return
  }
  if (result.Ok.kind !== asset.kind) {
    useToasts().error(`Choose a ${asset.kind} file`)
    return
  }
  editor.beginChange()
  editor.updateAsset(asset.id, { path: result.Ok.path, previewPath: undefined, name: result.Ok.name, duration: result.Ok.duration, width: result.Ok.width, height: result.Ok.height, hasAudio: result.Ok.hasAudio, size: result.Ok.size })
  if (result.Ok.needsProxy) void createPreview({ ...asset, path: result.Ok.path })
  useToasts().success('Media relinked', result.Ok.name)
}

const filmstrips = new Map<string, Promise<Filmstrip | null>>()
const waveforms = new Map<string, Promise<Uint8Array | null>>()

export function filmstripFor(asset: MediaAsset) {
  if (asset.kind !== 'video' || !asset.duration || !asset.width || !asset.height) return Promise.resolve(null)
  let request = filmstrips.get(asset.path)
  if (!request) {
    request = ipc.filmstrip(asset.path, asset.duration, asset.width, asset.height).catch(() => null)
    filmstrips.set(asset.path, request)
  }
  return request
}

export function waveformFor(asset: MediaAsset) {
  if (asset.kind === 'image' || asset.hasAudio === false) return Promise.resolve(null)
  let request = waveforms.get(asset.path)
  if (!request) {
    request = ipc.waveform(asset.path).catch(() => null)
    waveforms.set(asset.path, request)
  }
  return request
}

export function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`
  if (bytes < 1024 ** 3) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`
}

export function formatTime(seconds: number, fps?: number) {
  const safe = Math.max(0, seconds)
  const minutes = Math.floor(safe / 60)
  const secs = Math.floor(safe % 60)
  const base = `${String(minutes).padStart(2, '0')}:${String(secs).padStart(2, '0')}`
  if (!fps) return base
  const frames = Math.floor((safe % 1) * fps + 1e-6)
  return `${base}:${String(frames).padStart(2, '0')}`
}

export function formatDuration(seconds: number) {
  if (seconds < 60) return `${seconds.toFixed(seconds < 10 ? 1 : 0)}s`
  return formatTime(seconds)
}
