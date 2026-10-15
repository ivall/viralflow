import { clamp } from './engine/math'

/**
 * Smooth progress and time left for jobs that report rarely and in jumps, like
 * Whisper, which reports once per 20 s chunk of audio. Between reports the bar
 * creeps toward where the next report is expected, slowing down so it never
 * passes it, and the time left comes from the measured rate once there is one.
 * Times are in seconds.
 */
export class Pace {
  private readonly started: number
  private floor = 0
  private reportedAt: number
  private step: number
  private rate: number
  private shown = 0

  /** `expected`: estimated length of the whole job; `firstStep`: share of the job before the first report. */
  constructor(expected: number, firstStep: number, now: number) {
    this.started = now
    this.reportedAt = now
    this.rate = 1 / Math.max(0.5, expected)
    this.step = clamp(firstStep, 0.01, 1)
  }

  report(fraction: number, now: number) {
    const value = clamp(fraction, 0, 1)
    if (value <= this.floor) return
    // The last jump predicts the next one.
    this.step = Math.max(0.01, value - this.floor)
    this.floor = value
    this.reportedAt = now
    const elapsed = now - this.started
    if (elapsed > 0.2) this.rate = value / elapsed
  }

  /** Fraction to draw (0..1); never goes backwards. */
  value(now: number) {
    const ceiling = Math.min(1, this.floor + this.step)
    const expectedWait = this.step / this.rate
    const waited = Math.max(0, now - this.reportedAt) / Math.max(0.05, expectedWait)
    // Stops short of the ceiling: only a report may move the bar there.
    const creep = (ceiling - this.floor) * 0.95 * (1 - Math.exp(-1.5 * waited))
    this.shown = Math.max(this.shown, Math.min(0.995, this.floor + creep))
    return this.shown
  }

  /** Seconds left; the estimate is replaced by the measured rate after the first report. */
  remaining(now: number) {
    return Math.max(0, (1 - this.value(now)) / this.rate)
  }

  elapsed(now: number) {
    return Math.max(0, now - this.started)
  }
}

const LEARNED_KEY = 'viralflow:pace'

/** Seconds of work per second of audio measured on earlier runs of `job`, if any. */
export function learnedRate(job: string): number | null {
  try {
    const value = JSON.parse(localStorage.getItem(LEARNED_KEY) ?? '{}')[job]
    return typeof value === 'number' && value > 0 ? value : null
  } catch {
    return null
  }
}

/** Remembers how long `job` took for `units` seconds of audio, blended with earlier runs. */
export function learnRate(job: string, seconds: number, units: number) {
  if (!(seconds > 0 && units > 1)) return
  try {
    const stored = JSON.parse(localStorage.getItem(LEARNED_KEY) ?? '{}')
    const previous = typeof stored[job] === 'number' ? stored[job] : null
    const measured = seconds / units
    stored[job] = previous ? previous * 0.4 + measured * 0.6 : measured
    localStorage.setItem(LEARNED_KEY, JSON.stringify(stored))
  } catch {
    // Learning is a nicety; an unavailable storage only means a rougher first estimate.
  }
}
