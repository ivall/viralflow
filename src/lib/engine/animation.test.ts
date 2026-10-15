import { describe, expect, it } from 'vitest'
import type { TimelineItem } from '../types'
import { captionRun, motionSpan, resolveMotion } from './animation'
import { DEFAULT_TEXT_STYLE } from './text'

const word = (id: string, start: number, duration: number, content: string, highlightIndex: number): TimelineItem => ({
  id, trackId: 't', kind: 'text', name: content, start, duration,
  text: { ...DEFAULT_TEXT_STYLE, content, highlightIndex },
  animIn: { kind: 'pop', duration: 0.3 },
  animOut: { kind: 'fade', duration: 0.3 }
})

describe('word-highlight caption animation', () => {
  // "one two three" lit word by word, then a new caption.
  const items = [
    word('a', 0, 0.5, 'one two three', 0),
    word('b', 0.5, 0.5, 'one two three', 1),
    word('c', 1, 0.5, 'one two three', 2),
    word('d', 1.5, 1, 'next line', 0)
  ]

  it('groups the word clips of one caption', () => {
    expect(captionRun(items[1], items).map(item => item.id)).toEqual(['a', 'b', 'c'])
    expect(captionRun(items[3], items).map(item => item.id)).toEqual(['d'])
  })

  it('animates the caption in once and out once', () => {
    const span = motionSpan(items[1], items)
    expect(span.start).toBe(0)
    expect(span.duration).toBeCloseTo(1.5)
    // The middle word sits between the in and out animations: no motion.
    const middle = resolveMotion(span, 0.6 - span.start)
    expect(middle.scale).toBe(1)
    expect(middle.opacity).toBe(1)
    // The caption still pops in on its first word and fades out on its last.
    expect(resolveMotion(motionSpan(items[0], items), 0.05).opacity).toBeLessThan(1)
    expect(resolveMotion(motionSpan(items[2], items), 1.45).opacity).toBeLessThan(1)
  })

  it('leaves ordinary clips alone', () => {
    expect(motionSpan(items[3], items)).toBe(items[3])
  })
})
