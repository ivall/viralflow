import { describe, expect, it } from 'vitest'
import { Pace } from './pace'

describe('pace', () => {
  it('creeps toward the next report without passing it', () => {
    const pace = new Pace(100, 0.2, 0)
    const early = pace.value(5)
    const late = pace.value(500)
    expect(early).toBeGreaterThan(0)
    expect(late).toBeGreaterThan(early)
    expect(late).toBeLessThan(0.2)
  })

  it('never goes backwards and uses the measured rate', () => {
    const pace = new Pace(100, 0.5, 0)
    const before = pace.value(30)
    pace.report(0.1, 30)
    expect(pace.value(30)).toBeGreaterThanOrEqual(before)
    // 10% in 30 s: about 270 s left, not the 70 s the first guess implied.
    expect(pace.remaining(30)).toBeGreaterThan(200)
    pace.report(1, 60)
    expect(pace.value(60)).toBeCloseTo(0.995)
  })
})
