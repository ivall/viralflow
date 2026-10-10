import type { TextStyle, TimelineItem } from '../types'
import { rgba } from './color'

/**
 * Text is laid out and rasterised by the browser for both the preview and the
 * export, so typography (fonts, kerning, emoji) is identical in the final file.
 */

export const DEFAULT_TEXT_STYLE: TextStyle = {
  content: 'Your text',
  font: 'Inter',
  size: 84,
  weight: 800,
  italic: false,
  color: '#ffffff',
  align: 'center',
  letterSpacing: 0,
  lineHeight: 1.15,
  maxWidth: 86,
  uppercase: false,
  strokeColor: '#000000',
  strokeWidth: 0,
  backgroundColor: '#000000',
  backgroundOpacity: 0,
  backgroundPadding: 18,
  backgroundRadius: 14,
  shadowColor: '#000000',
  shadowOpacity: 0,
  shadowBlur: 12,
  shadowDistance: 4
}

export function textStyleOf(item: TimelineItem): TextStyle {
  return { ...DEFAULT_TEXT_STYLE, ...item.text }
}

export function fontString(style: TextStyle, scale = 1) {
  const family = `"${style.font.replace(/["\\]/g, '')}"`
  return `${style.italic ? 'italic ' : ''}${style.weight} ${style.size * scale}px ${family}, "Segoe UI Emoji", sans-serif`
}

interface Layout {
  lines: Array<{ text: string; width: number }>
  blockWidth: number
  lineHeight: number
  ascent: number
  descent: number
  margin: number
  padding: number
  width: number
  height: number
}

let measureContext: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null = null

function context2d() {
  if (!measureContext) {
    measureContext = typeof OffscreenCanvas !== 'undefined'
      ? new OffscreenCanvas(8, 8).getContext('2d')!
      : document.createElement('canvas').getContext('2d')!
  }
  return measureContext
}

function applyFont(ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D, style: TextStyle, scale: number) {
  ctx.font = fontString(style, scale)
  ctx.letterSpacing = `${style.letterSpacing * scale}px`
  ctx.textBaseline = 'alphabetic'
}

function wrap(ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D, content: string, maxWidth: number) {
  const lines: string[] = []
  for (const paragraph of content.split(/\r?\n/)) {
    const words = paragraph.split(/(\s+)/).filter(Boolean)
    let line = ''
    for (const word of words) {
      const candidate = line + word
      if (!line || ctx.measureText(candidate.trimEnd()).width <= maxWidth) {
        line = candidate
        continue
      }
      lines.push(line.trimEnd())
      line = word.trimStart()
    }
    // Hard-break words that are wider than the box on their own.
    const pending = [line.trimEnd()]
    while (pending.length) {
      const current = pending.shift()!
      if (ctx.measureText(current).width <= maxWidth || current.length <= 1) { lines.push(current); continue }
      let cut = current.length - 1
      while (cut > 1 && ctx.measureText(current.slice(0, cut)).width > maxWidth) cut--
      lines.push(current.slice(0, cut))
      pending.unshift(current.slice(cut))
    }
  }
  return lines
}

function layoutText(style: TextStyle, canvasWidth: number, scale: number): Layout {
  const ctx = context2d()
  applyFont(ctx, style, scale)
  const content = style.uppercase ? style.content.toLocaleUpperCase() : style.content
  const maxWidth = Math.max(20, (style.maxWidth / 100) * canvasWidth * scale)
  const texts = wrap(ctx, content.length ? content : ' ', maxWidth)
  const lines = texts.map(text => ({ text, width: ctx.measureText(text).width }))
  const probe = ctx.measureText('ÁgjÉ')
  const ascent = probe.fontBoundingBoxAscent || style.size * scale * 0.8
  const descent = probe.fontBoundingBoxDescent || style.size * scale * 0.2
  const lineHeight = Math.max(ascent + descent, style.size * scale * style.lineHeight)
  const blockWidth = Math.max(1, ...lines.map(line => line.width))
  const padding = style.backgroundOpacity > 0 ? style.backgroundPadding * scale : 0
  const shadow = style.shadowOpacity > 0 ? (style.shadowBlur + style.shadowDistance) * scale : 0
  const margin = Math.ceil(Math.max(style.strokeWidth * scale, shadow) + 2 * scale)
  const width = Math.ceil(blockWidth + 2 * padding + 2 * margin)
  const height = Math.ceil(lines.length * lineHeight + 2 * padding + 2 * margin)
  return { lines, blockWidth, lineHeight, ascent, descent, margin, padding, width, height }
}

/** Box of a text item in project pixels. */
export function measureTextBox(item: TimelineItem, canvasWidth: number) {
  const layout = layoutText(textStyleOf(item), canvasWidth, 1)
  return { width: layout.width, height: layout.height }
}

/** Renders the text box at `scale` × project pixels. */
export function rasterizeText(item: TimelineItem, canvasWidth: number, scale: number): HTMLCanvasElement {
  const style = textStyleOf(item)
  const layout = layoutText(style, canvasWidth, scale)
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, layout.width)
  canvas.height = Math.max(1, layout.height)
  const ctx = canvas.getContext('2d')!
  const inset = layout.margin
  if (layout.padding > 0) {
    ctx.fillStyle = rgba(style.backgroundColor, style.backgroundOpacity)
    ctx.beginPath()
    ctx.roundRect(inset, inset, layout.blockWidth + 2 * layout.padding, layout.lines.length * layout.lineHeight + 2 * layout.padding, style.backgroundRadius * scale)
    ctx.fill()
  }
  applyFont(ctx, style, scale)
  ctx.lineJoin = 'round'
  ctx.miterLimit = 2
  const originX = inset + layout.padding
  const originY = inset + layout.padding
  const shadow = style.shadowOpacity > 0
  layout.lines.forEach((line, index) => {
    const x = style.align === 'left' ? originX : style.align === 'right' ? originX + layout.blockWidth - line.width : originX + (layout.blockWidth - line.width) / 2
    const y = originY + index * layout.lineHeight + (layout.lineHeight - layout.ascent - layout.descent) / 2 + layout.ascent
    const setShadow = (on: boolean) => {
      ctx.shadowColor = on ? rgba(style.shadowColor, style.shadowOpacity) : 'transparent'
      ctx.shadowBlur = on ? style.shadowBlur * scale : 0
      ctx.shadowOffsetX = 0
      ctx.shadowOffsetY = on ? style.shadowDistance * scale : 0
    }
    if (style.strokeWidth > 0) {
      setShadow(shadow)
      ctx.strokeStyle = style.strokeColor
      ctx.lineWidth = style.strokeWidth * 2 * scale
      ctx.strokeText(line.text, x, y)
      setShadow(false)
    } else setShadow(shadow)
    ctx.fillStyle = style.color
    ctx.fillText(line.text, x, y)
    setShadow(false)
  })
  return canvas
}

export async function ensureFontsLoaded(items: TimelineItem[]) {
  if (typeof document === 'undefined' || !document.fonts) return
  const fonts = new Set(items.filter(item => item.kind === 'text').map(item => fontString(textStyleOf(item))))
  await Promise.all([...fonts].map(font => document.fonts.load(font).catch(() => [])))
}
