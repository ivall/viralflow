import type { TimelineItem } from '../types'

export const MIN_CLIP_DURATION = .08

export type TimelineMoveMode = 'move' | 'trim-start' | 'trim-end'

export interface TimelineSpan {
  id: string
  start: number
  duration: number
}

export interface TimelineTrackSpans {
  id: string
  items: TimelineSpan[]
}

export interface TimelineSelectionSpan extends TimelineSpan {
  trackId: string
}

function endOf(span: Pick<TimelineSpan, 'start' | 'duration'>) {
  return span.start + span.duration
}

function sortedObstacles(items: TimelineSpan[], ignored: Set<string>) {
  return items
    .filter(item => !ignored.has(item.id))
    .map(item => ({ ...item, start: Math.max(0, item.start), duration: Math.max(MIN_CLIP_DURATION, item.duration) }))
    .sort((a, b) => a.start - b.start || a.id.localeCompare(b.id))
}

function openGaps(items: TimelineSpan[], duration: number, ignored: Set<string>) {
  const gaps: Array<{ start: number; end: number }> = []
  let cursor = 0
  for (const item of sortedObstacles(items, ignored)) {
    if (item.start > cursor) gaps.push({ start: cursor, end: Math.min(duration, item.start) })
    cursor = Math.max(cursor, endOf(item))
    if (cursor >= duration) break
  }
  if (cursor < duration) gaps.push({ start: cursor, end: duration })
  return gaps
}

function nearestStart(gaps: Array<{ start: number; end: number }>, desiredStart: number, width: number) {
  let best: number | null = null
  let bestDistance = Number.POSITIVE_INFINITY
  for (const gap of gaps) {
    if (gap.end - gap.start + 1e-6 < width) continue
    const start = Math.min(gap.end - width, Math.max(gap.start, desiredStart))
    const distance = Math.abs(start - desiredStart)
    if (distance < bestDistance - 1e-6 || (Math.abs(distance - bestDistance) <= 1e-6 && (best === null || start < best))) {
      best = start
      bestDistance = distance
    }
  }
  return best
}

/** Finds the closest legal position without changing the requested clip length. */
export function placeTimelineSpan(items: TimelineSpan[], candidate: TimelineSpan, projectDuration: number, ignoreIds: Iterable<string> = []) {
  const duration = Math.min(Math.max(MIN_CLIP_DURATION, candidate.duration), Math.max(MIN_CLIP_DURATION, projectDuration))
  const ignored = new Set(ignoreIds)
  ignored.add(candidate.id)
  const start = nearestStart(openGaps(items, projectDuration, ignored), Math.max(0, candidate.start), duration)
  return start === null ? null : { ...candidate, start, duration }
}

/** Moves an arbitrary multi-track selection with one shared delta. */
export function placeTimelineSelection(tracks: TimelineTrackSpans[], candidates: TimelineSelectionSpan[], projectDuration: number) {
  if (!candidates.length) return []
  const selectedIds = new Set(candidates.map(item => item.id))
  const obstacles = new Map(tracks.map(track => [track.id, sortedObstacles(track.items, selectedIds)]))
  const lower = -Math.min(...candidates.map(item => item.start))
  const upper = projectDuration - Math.max(...candidates.map(endOf))
  if (lower > upper + 1e-6) return null

  const forbidden: Array<{ start: number; end: number }> = []
  for (const candidate of candidates) {
    for (const obstacle of obstacles.get(candidate.trackId) || []) {
      const start = obstacle.start - endOf(candidate)
      const end = endOf(obstacle) - candidate.start
      if (end > lower + 1e-6 && start < upper - 1e-6) forbidden.push({ start, end })
    }
  }

  const isValid = (delta: number) => {
    if (delta < lower - 1e-6 || delta > upper + 1e-6) return false
    return candidates.every((candidate) => {
      const start = candidate.start + delta
      const end = endOf(candidate) + delta
      return (obstacles.get(candidate.trackId) || []).every(obstacle => start >= endOf(obstacle) - 1e-6 || end <= obstacle.start + 1e-6)
    })
  }

  const points = [Math.min(upper, Math.max(lower, 0)), lower, upper]
  for (const interval of forbidden) points.push(Math.min(upper, Math.max(lower, interval.start)), Math.min(upper, Math.max(lower, interval.end)))
  const delta = [...new Set(points.map(value => Math.round(value * 1e6) / 1e6))]
    .sort((a, b) => Math.abs(a) - Math.abs(b) || a - b)
    .find(isValid)
  if (delta === undefined) return null
  return candidates.map(item => ({ ...item, start: item.start + delta }))
}

/** Constrains one trim edge against its immediate neighbour. */
export function trimTimelineSpan(items: TimelineSpan[], current: TimelineSpan, candidate: TimelineSpan, edge: 'start' | 'end', projectDuration: number) {
  const obstacles = sortedObstacles(items, new Set([current.id]))
  if (edge === 'start') {
    const fixedEnd = Math.min(projectDuration, endOf(candidate))
    const previousEnd = obstacles
      .filter(item => item.start < current.start + 1e-6)
      .reduce((value, item) => Math.max(value, endOf(item)), 0)
    const start = Math.min(fixedEnd - MIN_CLIP_DURATION, Math.max(previousEnd, candidate.start))
    return { ...candidate, start, duration: Math.max(MIN_CLIP_DURATION, fixedEnd - start) }
  }

  const nextStart = obstacles
    .filter(item => item.start >= endOf(current) - 1e-6)
    .reduce((value, item) => Math.min(value, item.start), projectDuration)
  const duration = Math.max(MIN_CLIP_DURATION, Math.min(candidate.duration, nextStart - current.start))
  return { ...candidate, start: current.start, duration }
}

/** Changes a clip length and ripples only neighbours that would otherwise overlap. */
export function rippleTimelineResize(items: TimelineSpan[], itemId: string, nextDuration: number) {
  const current = items.find(item => item.id === itemId)
  if (!current) return { duration: Math.max(MIN_CLIP_DURATION, nextDuration), starts: new Map<string, number>(), end: 0 }
  const duration = Math.max(MIN_CLIP_DURATION, nextDuration)
  const starts = new Map<string, number>()
  let cursor = current.start + duration
  const following = items
    .filter(item => item.id !== itemId && item.start >= current.start - 1e-6)
    .sort((a, b) => a.start - b.start || a.id.localeCompare(b.id))
  for (const item of following) {
    const start = item.start < cursor ? cursor : item.start
    if (Math.abs(start - item.start) > 1e-6) starts.set(item.id, start)
    cursor = start + Math.max(MIN_CLIP_DURATION, item.duration)
  }
  return { duration, starts, end: cursor }
}

/** Repairs legacy/imported tracks by keeping order and shifting collisions to the right. */
export function repairTimelineLayout(items: TimelineSpan[]) {
  const starts = new Map<string, number>()
  let cursor = 0
  for (const item of [...items].sort((a, b) => a.start - b.start || a.id.localeCompare(b.id))) {
    const start = Math.max(0, item.start, cursor)
    starts.set(item.id, start)
    cursor = start + Math.max(MIN_CLIP_DURATION, item.duration)
  }
  return { starts, end: cursor }
}

export function timelineHasOverlap(items: Pick<TimelineItem, 'id' | 'start' | 'duration'>[]) {
  const sorted = [...items].sort((a, b) => a.start - b.start)
  return sorted.some((item, index) => index > 0 && item.start < endOf(sorted[index - 1]) - 1e-6)
}
