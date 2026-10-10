import type { CaptionCue } from './types'

function parseTimestamp(value: string) {
  const match = value.trim().match(/^(\d+):(\d{1,2}):(\d{1,2})[,.](\d{1,3})$/)
  if (!match) return null
  const [, hours, minutes, seconds, millis] = match
  return Number(hours) * 3600 + Number(minutes) * 60 + Number(seconds) + Number(millis.padEnd(3, '0')) / 1000
}

/** Parses SubRip (.srt) and simple WebVTT files. */
export function parseSubtitles(text: string): CaptionCue[] {
  const cues: CaptionCue[] = []
  const blocks = text.replace(/\r/g, '').replace(/^WEBVTT[^\n]*\n/, '').split(/\n{2,}/)
  for (const block of blocks) {
    const lines = block.split('\n').filter(line => line.trim().length)
    const timingIndex = lines.findIndex(line => line.includes('-->'))
    if (timingIndex < 0) continue
    const [from, to] = lines[timingIndex].split('-->').map(part => part.trim().split(/\s+/)[0])
    const start = parseTimestamp(from.includes(':') && from.split(':').length === 2 ? `0:${from}` : from)
    const end = parseTimestamp(to.includes(':') && to.split(':').length === 2 ? `0:${to}` : to)
    const content = lines.slice(timingIndex + 1).join('\n').replace(/<[^>]+>/g, '').trim()
    if (start === null || end === null || end <= start || !content) continue
    cues.push({ text: content, start, duration: end - start })
  }
  return cues.sort((a, b) => a.start - b.start)
}

function timestamp(seconds: number) {
  const totalMillis = Math.max(0, Math.round(seconds * 1000))
  const hours = Math.floor(totalMillis / 3_600_000)
  const minutes = Math.floor((totalMillis % 3_600_000) / 60_000)
  const secs = Math.floor((totalMillis % 60_000) / 1000)
  const millis = totalMillis % 1000
  return `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(secs).padStart(2, '0')},${String(millis).padStart(3, '0')}`
}

export function formatSrt(cues: CaptionCue[]) {
  return cues
    .slice()
    .sort((a, b) => a.start - b.start)
    .map((cue, index) => `${index + 1}\n${timestamp(cue.start)} --> ${timestamp(cue.start + cue.duration)}\n${cue.text}\n`)
    .join('\n')
}
