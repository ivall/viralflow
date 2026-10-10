import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { migrateProject } from '../migrate'
import { resolveScene } from './scene'

/**
 * The preview (TypeScript) and the exporter (Rust) must compute identical layer
 * states. This test writes the reference values that src-tauri/tests/parity.rs
 * checks, and guards them against unintended changes on the TS side.
 */
const fixtures = fileURLToPath(new URL('../../../src-tauri/tests/fixtures/', import.meta.url))
const SAMPLES = [0, 0.3, 0.75, 1.2, 1.9, 2.6, 2.75, 3.05, 3.3, 3.45, 4.1, 5.2, 5.6, 6.1, 6.4, 7.3, 7.8, 8.1, 8.25, 9.5]

function computeExpected() {
  const project = migrateProject(JSON.parse(readFileSync(`${fixtures}parity-project.json`, 'utf8')))
  const assets = new Map(project.assets.map(asset => [asset.id, asset]))
  const context = { assets, measureText: () => ({ width: 100, height: 50 }) }
  return SAMPLES.map(time => ({
    time,
    layers: resolveScene(project, time, context).map(layer => ({
      id: layer.item.id,
      boxW: layer.boxW,
      boxH: layer.boxH,
      mediaTime: layer.mediaTime ?? null,
      cx: layer.cx,
      cy: layer.cy,
      scale: layer.scale,
      rotation: layer.rotation,
      opacity: layer.opacity,
      blur: layer.blur,
      clipLeft: layer.clipLeft,
      clipRight: layer.clipRight,
      vignette: layer.vignette,
      grain: layer.grain,
      glitch: layer.glitch,
      matrix: layer.matrix
    }))
  }))
}

describe('preview/export parity', () => {
  it('matches the reference scene states', () => {
    const expected = computeExpected()
    const path = `${fixtures}parity-expected.json`
    if (!existsSync(path) || process.env.UPDATE_PARITY) writeFileSync(path, JSON.stringify(expected, null, 1))
    const stored = JSON.parse(readFileSync(path, 'utf8'))
    expect(JSON.parse(JSON.stringify(expected))).toEqual(stored)
    expect(expected.some(sample => sample.layers.length >= 3)).toBe(true)
  })
})
