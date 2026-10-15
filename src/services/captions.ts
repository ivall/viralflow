import { useEditor } from '@/stores/editor'
import { useSettings } from '@/stores/settings'
import type { CaptionCue, TextStyle, TimelineItem } from '@/lib/types'
import { learnedRate, learnRate, Pace } from '@/lib/pace'
import { MIN_CLIP_DURATION } from '@/lib/timeline/placement'
import { ipc, onEvent, type Transcript } from './ipc'

export interface CaptionOptions {
  language: string
  maxWords: number
  style?: Partial<TextStyle>
  y?: number
  /** Receives the progress of the transcription as soon as it starts. */
  onPace?: (pace: Pace) => void
}

/** First guess of seconds of work per second of audio, before a run has been measured. */
const DEFAULT_RATES: Record<string, number> = { tiny: 0.1, base: 0.2, small: 0.6, medium: 1.6, 'large-v3-turbo': 1.2, openrouter: 0.15 }
/** Whisper hands back progress once per chunk of this many seconds. */
const WHISPER_CHUNK = 20

const clock = () => performance.now() / 1000

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
  const settings = useSettings().value
  const audio = item.duration * speed
  const cloud = settings.transcriptionProvider === 'openrouter'
  const job = cloud ? 'openrouter' : settings.whisperModel
  const rate = learnedRate(`transcribe:${job}`) ?? DEFAULT_RATES[job] ?? 0.5
  // Locally the words are found in two passes over the audio (sentences, then word timings).
  const pace = new Pace(2 + audio * rate, cloud ? 0.95 : Math.min(1, WHISPER_CHUNK / Math.max(1, audio)) / 2, clock())
  options.onPace?.(pace)
  const unlisten = cloud ? null : await onEvent<{ pass: number; progress: number }>('transcribe-progress', event => pace.report((event.pass + event.progress) / 2, clock()))
  let transcript: Transcript
  try {
    transcript = await ipc.transcribe({
      path: asset.path,
      offset: item.sourceIn ?? 0,
      duration: audio,
      language: options.language || settings.captionLanguage || 'auto',
      options: { maxWords: options.maxWords }
    })
  } finally {
    unlisten?.()
  }
  // Without speech the second pass is skipped, which would teach a too-short time.
  if (transcript.cues.length) learnRate(`transcribe:${job}`, pace.elapsed(clock()), audio)
  if (!transcript.cues.length) return { count: 0, alignment: transcript.alignment }
  const highlighted = options.style?.highlightMode && options.style.highlightMode !== 'none'
  const cues = (highlighted && highlightCues(transcript.cues, transcript.words)) || transcript.cues
  const count = editor.applyCaptions(cues, { sourceId: item.id, offset: item.start, speed, style: options.style, y: options.y })
  return { count, alignment: transcript.alignment }
}

const normalizeWord = (word: string) => word.toLocaleLowerCase().replace(/[^\p{L}\p{N}]/gu, '')

/**
 * Splits every caption into one cue per spoken word: the caption text stays the
 * same and only the highlighted word moves on. Returns null when the word
 * timings cannot be matched to the captions.
 */
export function highlightCues(cues: CaptionCue[], words: Transcript['words']): CaptionCue[] | null {
  if (!words.length) return null
  const result: CaptionCue[] = []
  let cursor = 0
  for (const cue of cues) {
    const parts = cue.text.split(/\s+/).filter(Boolean)
    const wanted = parts.map(normalizeWord)
    let found = -1
    for (let index = cursor; index + parts.length <= words.length && index <= cursor + 8; index++) {
      if (wanted.every((word, offset) => normalizeWord(words[index + offset].word) === word)) { found = index; break }
    }
    if (found < 0) return null
    const timed = words.slice(found, found + parts.length)
    cursor = found + parts.length
    const end = cue.start + cue.duration
    // Each word is lit from its own start until the next word starts.
    const starts = timed.map((word, index) => index === 0 ? cue.start : Math.min(end, Math.max(cue.start, word.start)))
    for (let index = 1; index < starts.length; index++) starts[index] = Math.max(starts[index], starts[index - 1] + MIN_CLIP_DURATION)
    for (let index = 0; index < parts.length; index++) {
      const start = starts[index]
      const next = index + 1 < parts.length ? starts[index + 1] : end
      const stop = next > end - MIN_CLIP_DURATION ? end : next
      if (stop - start < MIN_CLIP_DURATION) break
      result.push({ text: cue.text, start, duration: stop - start, highlightIndex: index })
      if (stop >= end) break
    }
  }
  return result
}

export function captionCues(): CaptionCue[] {
  const editor = useEditor()
  return editor.tracks
    .filter(track => track.kind === 'text' && !track.hidden)
    .flatMap(track => track.items.filter(item => item.kind === 'text' && !item.hidden && item.text?.content.trim()))
    .filter(item => item.groupId?.startsWith('captions-') || editor.tracks.find(track => track.id === item.trackId)?.name.startsWith('Captions'))
    .map(item => ({ text: item.text!.content, start: item.start, duration: item.duration, highlightIndex: item.text!.highlightIndex }))
    .sort((left, right) => left.start - right.start)
    // Word-highlight captions are one clip per word; subtitles want one cue per caption.
    .reduce<CaptionCue[]>((cues, cue) => {
      const previous = cues.at(-1)
      if (previous && cue.highlightIndex > 0 && previous.text === cue.text && Math.abs(previous.start + previous.duration - cue.start) < 0.02) previous.duration = cue.start + cue.duration - previous.start
      else cues.push({ text: cue.text, start: cue.start, duration: cue.duration })
      return cues
    }, [])
}
