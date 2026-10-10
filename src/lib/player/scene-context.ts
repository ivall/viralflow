import type { SceneContext } from '../engine/scene'
import { measureTextBox } from '../engine/text'
import type { MediaAsset, TimelineItem } from '../types'

const textBoxes = new Map<string, { key: string; box: { width: number; height: number } }>()
let fontVersion = 0

if (typeof document !== 'undefined' && document.fonts) {
  document.fonts.addEventListener('loadingdone', () => { fontVersion++ })
}

export function measureCached(item: TimelineItem, canvasWidth: number) {
  const key = `${fontVersion}|${canvasWidth}|${JSON.stringify(item.text)}`
  const cached = textBoxes.get(item.id)
  if (cached?.key === key) return cached.box
  const box = measureTextBox(item, canvasWidth)
  textBoxes.set(item.id, { key, box })
  if (textBoxes.size > 2000) textBoxes.delete(textBoxes.keys().next().value!)
  return box
}

export function sceneContext(assets: Map<string, MediaAsset>, canvasWidth: number): SceneContext {
  return { assets, measureText: item => measureCached(item, canvasWidth) }
}

export function currentFontVersion() {
  return fontVersion
}
