import { describe, expect, it } from 'vitest'
import { applyEasing, cubicBezier, EASING_CURVES } from './math'

describe('cubic-bezier easing', () => {
  it('starts at 0 and ends at 1', () => {
    for (const curve of Object.values(EASING_CURVES)) {
      expect(cubicBezier(curve, 0)).toBeCloseTo(0, 9)
      expect(cubicBezier(curve, 1)).toBeCloseTo(1, 9)
    }
  })

  it('follows the CSS curves', () => {
    // CSS ease-in-out at x = 0.25 is about 0.129.
    expect(cubicBezier([0.42, 0, 0.58, 1], 0.25)).toBeCloseTo(0.129, 3)
    expect(cubicBezier(EASING_CURVES.linear, 0.4)).toBeCloseTo(0.4, 6)
  })

  it('overshoots with back-out and dips with back-in', () => {
    expect(Math.max(...[0.5, 0.6, 0.7, 0.8].map(x => applyEasing(x, 'back-out')))).toBeGreaterThan(1)
    expect(applyEasing(0.2, 'back-in')).toBeLessThan(0)
  })

  it('treats a missing custom curve as linear', () => {
    expect(applyEasing(0.3, 'custom')).toBe(0.3)
    // Both coordinates share one polynomial, so this curve is a straight line too.
    expect(applyEasing(0.3, 'custom', [0, 0, 1, 1])).toBeCloseTo(0.3, 6)
  })
})
