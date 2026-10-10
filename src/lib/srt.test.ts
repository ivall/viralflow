import { describe, expect, it } from 'vitest'
import { formatSrt, parseSubtitles } from './srt'

describe('subtitles', () => {
  it('round-trips SRT', () => {
    const cues = [{ text: 'Hello there', start: 1.2, duration: 1.5 }, { text: 'Second\nline', start: 3, duration: 0.75 }]
    const parsed = parseSubtitles(formatSrt(cues))
    expect(parsed).toHaveLength(2)
    expect(parsed[0].start).toBeCloseTo(1.2)
    expect(parsed[0].duration).toBeCloseTo(1.5)
    expect(parsed[1].text).toBe('Second\nline')
  })

  it('reads WebVTT with short timestamps and tags', () => {
    const parsed = parseSubtitles('WEBVTT\n\n00:01.500 --> 00:03.000 align:start\n<b>Bold</b> words\n')
    expect(parsed).toEqual([{ text: 'Bold words', start: 1.5, duration: 1.5 }])
  })
})
