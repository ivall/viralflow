import { useEditor } from '@/stores/editor'
import { useSettings } from '@/stores/settings'
import type { CaptionCue, TextStyle, TimelineItem } from '@/lib/types'
import { ipc } from './ipc'

export interface CaptionOptions {
  language: string
  maxWords: number
  style?: Partial<TextStyle>
  y?: number
}

/** Clips whose sound can be transcribed. */
export function speechSources(): TimelineItem[] {
  const editor = useEditor()
  return editor.tracks.flatMap(track => track.items.filter(item => {
    if (item.hidden || !item.assetId || editor.offline.has(item.assetId)) return false
    if (item.kind === 'audio') return true
    return item.kind === 'video' && editor.assetsById.get(item.assetId)?.hasAudio !== false
  })).sort((a, b) => Number(b.kind === 'audio') - Number(a.kind === 'audio') || b.duration - a.duration)
}

/** Transcribes a clip and places the captions on a caption track. Returns the number of captions. */
export async function generateCaptions(itemId: string, options: CaptionOptions) {
  const editor = useEditor()
  const found = editor.find(itemId)
  if (!found) throw new Error('The clip no longer exists.')
  const { item } = found
  const asset = item.assetId ? editor.assetsById.get(item.assetId) : undefined
  if (!asset) throw new Error('This clip has no media file.')
  const speed = item.speed || 1
  const transcript = await ipc.transcribe({
    path: asset.path,
    offset: item.sourceIn ?? 0,
    duration: item.duration * speed,
    language: options.language || useSettings().value.captionLanguage || 'auto',
    options: { maxWords: options.maxWords }
  })
  if (!transcript.cues.length) return { count: 0, alignment: transcript.alignment }
  const count = editor.applyCaptions(transcript.cues, { sourceId: item.id, offset: item.start, speed, style: options.style, y: options.y })
  return { count, alignment: transcript.alignment }
}

export function captionCues(): CaptionCue[] {
  const editor = useEditor()
  return editor.tracks
    .filter(track => track.kind === 'text' && !track.hidden)
    .flatMap(track => track.items.filter(item => item.kind === 'text' && !item.hidden && item.text?.content.trim()))
    .filter(item => item.groupId?.startsWith('captions-') || editor.tracks.find(track => track.id === item.trackId)?.name.startsWith('Captions'))
    .map(item => ({ text: item.text!.content, start: item.start, duration: item.duration }))
}
