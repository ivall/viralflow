import { toRaw } from 'vue'
import { measureTextBox, rasterizeText, ensureFontsLoaded } from '@/lib/engine/text'
import type { Project, TimelineItem } from '@/lib/types'
import type { TextRaster } from './ipc'

/** Output pixels per project pixel for the chosen export resolution. */
export function exportScale(project: Project) {
  return project.export.resolution / Math.min(project.width, project.height)
}

export function outputSize(project: Project) {
  const k = exportScale(project)
  const even = (value: number) => Math.max(2, Math.round(value / 2) * 2)
  return { width: even(project.width * k), height: even(project.height * k) }
}

function peakScale(item: TimelineItem) {
  let peak = item.transform?.scale ?? 1
  for (const keyframe of item.keyframes ?? []) peak = Math.max(peak, keyframe.transform.scale)
  if (item.animIn?.kind === 'zoom-out' || item.animOut?.kind === 'grow') peak *= 1.8
  else if (item.animIn?.kind && item.animIn.kind !== 'none') peak *= 1.16
  return peak
}

/** Renders every visible text item to a PNG at the size it will appear in the export. */
export async function rasterizeTexts(project: Project): Promise<TextRaster[]> {
  const items = project.tracks.filter(track => !track.hidden).flatMap(track => track.items.filter(item => item.kind === 'text' && !item.hidden))
  await ensureFontsLoaded(items)
  const k = exportScale(project)
  const rasters: TextRaster[] = []
  for (const item of items) {
    const box = measureTextBox(item, project.width)
    const scale = k * Math.min(3, Math.max(1, peakScale(item) * 1.05))
    const canvas = rasterizeText(item, project.width, scale)
    rasters.push({ itemId: item.id, png: canvas.toDataURL('image/png'), width: box.width, height: box.height })
    // Yield so the UI stays responsive with hundreds of captions.
    if (rasters.length % 25 === 0) await new Promise(resolve => setTimeout(resolve))
  }
  return rasters
}

export function exportableProject(project: Project): Project {
  return JSON.parse(JSON.stringify(toRaw(project)))
}

export function estimateMegabytes(project: Project, duration: number) {
  const { width, height } = outputSize(project)
  const pixelsPerSecond = width * height * project.export.fps
  const bitsPerPixel = { draft: 0.05, standard: 0.08, high: 0.12, ultra: 0.2 }[project.export.quality] * (project.export.codec === 'h264' ? 1 : 0.65)
  return Math.max(1, Math.round((pixelsPerSecond * bitsPerPixel * duration) / 8 / 1024 / 1024 + duration * 0.024))
}
