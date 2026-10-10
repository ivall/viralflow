import type { ColorMatrix } from '../engine/color'
import { GLITCH_BANDS } from '../engine/effects'
import { hash01 } from '../engine/math'
import type { Layer } from '../engine/scene'
import { rasterizeText } from '../engine/text'
import type { BlendMode, MediaAsset, Project, TimelineItem } from '../types'
import type { MediaPool } from './media-pool'

/**
 * Draws resolved scene layers onto a 2D canvas. The geometry and order of
 * operations mirror src-tauri/src/render/composite.rs.
 */

const BLEND: Record<BlendMode, GlobalCompositeOperation> = {
  normal: 'source-over', multiply: 'multiply', screen: 'screen', overlay: 'overlay', darken: 'darken', lighten: 'lighten',
  'color-dodge': 'color-dodge', 'color-burn': 'color-burn', 'hard-light': 'hard-light', 'soft-light': 'soft-light',
  difference: 'difference', exclusion: 'exclusion'
}

const SVG_NS = 'http://www.w3.org/2000/svg'

/** SVG feColorMatrix filters referenced from ctx.filter = url(#…). */
class ColorFilters {
  private svg: SVGSVGElement
  private ids = new Map<string, string>()
  private order: string[] = []
  private counter = 0

  constructor() {
    this.svg = document.createElementNS(SVG_NS, 'svg')
    this.svg.setAttribute('aria-hidden', 'true')
    this.svg.style.cssText = 'position:absolute;width:0;height:0;overflow:hidden'
    document.body.appendChild(this.svg)
  }

  url(matrix: ColorMatrix) {
    const key = matrix.map(value => value.toFixed(3)).join(' ')
    let id = this.ids.get(key)
    if (!id) {
      id = `vf-cm-${this.counter++}`
      const filter = document.createElementNS(SVG_NS, 'filter')
      filter.id = id
      filter.setAttribute('color-interpolation-filters', 'sRGB')
      const node = document.createElementNS(SVG_NS, 'feColorMatrix')
      node.setAttribute('type', 'matrix')
      node.setAttribute('values', key)
      filter.appendChild(node)
      this.svg.appendChild(filter)
      this.ids.set(key, id)
      this.order.push(key)
      if (this.order.length > 300) {
        const old = this.order.shift()!
        const oldId = this.ids.get(old)
        this.ids.delete(old)
        if (oldId) this.svg.querySelector(`#${oldId}`)?.remove()
      }
    }
    return `url(#${id})`
  }

  destroy() {
    this.svg.remove()
  }
}

/** Caches text rasters; a raster is reused until its style or scale changes. */
class TextCache {
  private entries = new Map<string, { key: string; canvas: HTMLCanvasElement }>()
  version = 0

  get(item: TimelineItem, canvasWidth: number, scale: number) {
    const bucket = Math.max(0.25, Math.ceil(scale * 4) / 4)
    const key = `${this.version}|${bucket}|${canvasWidth}|${JSON.stringify(item.text)}`
    const cached = this.entries.get(item.id)
    if (cached?.key === key) return cached.canvas
    const canvas = rasterizeText(item, canvasWidth, bucket)
    this.entries.set(item.id, { key, canvas })
    if (this.entries.size > 400) this.entries.delete(this.entries.keys().next().value!)
    return canvas
  }

  invalidate() {
    this.version++
  }
}

function noiseTile() {
  const canvas = document.createElement('canvas')
  canvas.width = 128
  canvas.height = 128
  const ctx = canvas.getContext('2d')!
  const data = ctx.createImageData(128, 128)
  for (let index = 0; index < data.data.length; index += 4) {
    const value = Math.floor(hash01(index * 13 + 7) * 255)
    data.data[index] = data.data[index + 1] = data.data[index + 2] = value
    data.data[index + 3] = 255
  }
  ctx.putImageData(data, 0, 0)
  return canvas
}

export class PreviewRenderer {
  private colorFilters = new ColorFilters()
  readonly text = new TextCache()
  private noise: HTMLCanvasElement | null = null

  /**
   * @param scale canvas pixels per project pixel
   */
  draw(ctx: CanvasRenderingContext2D, project: Project, layers: Layer[], pool: MediaPool, assets: Map<string, MediaAsset>, scale: number, time: number) {
    const { width, height } = ctx.canvas
    ctx.save()
    ctx.setTransform(1, 0, 0, 1, 0, 0)
    ctx.globalAlpha = 1
    ctx.globalCompositeOperation = 'source-over'
    ctx.filter = 'none'
    ctx.fillStyle = project.background
    ctx.fillRect(0, 0, width, height)
    ctx.restore()
    for (const layer of layers) this.drawLayer(ctx, project, layer, pool, assets, scale, time)
  }

  private source(layer: Layer, pool: MediaPool, assets: Map<string, MediaAsset>, project: Project, scale: number): CanvasImageSource | null {
    const item = layer.item
    if (item.kind === 'video') return pool.video(item.id)
    if (item.kind === 'image') {
      const asset = item.assetId ? assets.get(item.assetId) : undefined
      return asset ? pool.image(asset) : null
    }
    if (item.kind === 'text') return this.text.get(item, project.width, scale * Math.max(1, layer.scale))
    return null
  }

  private applyTransform(ctx: CanvasRenderingContext2D, layer: Layer, scale: number) {
    ctx.translate(layer.cx * scale, layer.cy * scale)
    ctx.rotate((layer.rotation * Math.PI) / 180)
    ctx.scale(layer.scale * (layer.flipX ? -1 : 1), layer.scale * (layer.flipY ? -1 : 1))
  }

  private clipBox(ctx: CanvasRenderingContext2D, layer: Layer, w: number, h: number, scale: number, band?: [number, number]) {
    const needsRound = layer.radius > 0.01
    if (needsRound) {
      ctx.beginPath()
      ctx.roundRect(-w / 2, -h / 2, w, h, Math.min(layer.radius * scale, w / 2, h / 2))
      ctx.clip()
    }
    if (layer.clipLeft > 0.0001 || layer.clipRight < 0.9999 || band) {
      const left = -w / 2 + layer.clipLeft * w
      const right = -w / 2 + layer.clipRight * w
      const top = band ? -h / 2 + band[0] : -h / 2
      const bottom = band ? -h / 2 + band[1] : h / 2
      ctx.beginPath()
      ctx.rect(left, top, Math.max(0, right - left), Math.max(0, bottom - top))
      ctx.clip()
    }
  }

  private drawLayer(ctx: CanvasRenderingContext2D, project: Project, layer: Layer, pool: MediaPool, assets: Map<string, MediaAsset>, scale: number, time: number) {
    const item = layer.item
    const w = layer.boxW * scale
    const h = layer.boxH * scale
    if (w < 0.5 || h < 0.5) return

    if (item.kind === 'shape' && item.shape?.kind === 'blur') {
      ctx.save()
      this.applyTransform(ctx, layer, scale)
      ctx.beginPath()
      ctx.roundRect(-w / 2, -h / 2, w, h, Math.min((item.radius ?? 0) * scale, w / 2, h / 2))
      ctx.clip()
      ctx.setTransform(1, 0, 0, 1, 0, 0)
      ctx.globalAlpha = layer.opacity
      ctx.filter = `blur(${Math.max(1, item.shape.blur) * scale}px)`
      ctx.drawImage(ctx.canvas, 0, 0)
      ctx.restore()
      return
    }

    ctx.save()
    ctx.globalAlpha = layer.opacity
    ctx.globalCompositeOperation = BLEND[layer.blend] ?? 'source-over'
    this.applyTransform(ctx, layer, scale)
    const filters: string[] = []
    if (layer.matrix) filters.push(this.colorFilters.url(layer.matrix))
    if (layer.blur > 0.3) filters.push(`blur(${layer.blur * scale}px)`)
    ctx.filter = filters.length ? filters.join(' ') : 'none'

    if (item.kind === 'shape') {
      this.clipBox(ctx, layer, w, h, scale)
      ctx.fillStyle = item.shape?.color ?? '#ffffff'
      ctx.beginPath()
      if (item.shape?.kind === 'circle') ctx.ellipse(0, 0, w / 2, h / 2, 0, 0, Math.PI * 2)
      else ctx.roundRect(-w / 2, -h / 2, w, h, item.shape?.kind === 'line' ? h / 2 : Math.min((item.radius ?? 0) * scale, w / 2, h / 2))
      ctx.fill()
      ctx.restore()
      return
    }

    const source = this.source(layer, pool, assets, project, scale)
    if (!source) {
      if (item.kind === 'video' || item.kind === 'image') {
        ctx.filter = 'none'
        ctx.fillStyle = 'rgba(255,255,255,0.04)'
        ctx.fillRect(-w / 2, -h / 2, w, h)
      }
      ctx.restore()
      return
    }

    const drawSource = (dx = 0) => {
      if (layer.source) ctx.drawImage(source, layer.source.sx, layer.source.sy, layer.source.sw, layer.source.sh, -w / 2 + dx, -h / 2, w, h)
      else ctx.drawImage(source, -w / 2 + dx, -h / 2, w, h)
    }

    if (layer.glitch.length) {
      const band = h / GLITCH_BANDS
      layer.glitch.forEach((offset, index) => {
        ctx.save()
        this.clipBox(ctx, layer, w, h, scale, [index * band, (index + 1) * band])
        drawSource((offset / 100) * w)
        ctx.restore()
      })
    } else {
      this.clipBox(ctx, layer, w, h, scale)
      drawSource()
    }

    if (layer.vignette > 0.001 || layer.grain > 0 || layer.scanlines) {
      ctx.filter = 'none'
      ctx.globalCompositeOperation = 'source-over'
      if (layer.vignette > 0.001) {
        // Same elliptical smoothstep falloff as pixels::apply_vignette.
        const radius = (w / 2) * Math.SQRT2
        const gradient = ctx.createRadialGradient(0, 0, 0, 0, 0, radius)
        for (const [stop, t] of [[0.35, 0], [0.5, 0.156], [0.675, 0.5], [0.85, 0.844], [1, 1]] as const) {
          gradient.addColorStop(stop, `rgba(0,0,0,${0.85 * layer.vignette * t})`)
        }
        gradient.addColorStop(0, 'rgba(0,0,0,0)')
        ctx.save()
        ctx.scale(1, h / Math.max(1, w))
        ctx.fillStyle = gradient
        ctx.fillRect(-w / 2, -w / 2, w, w)
        ctx.restore()
      }
      if (layer.grain > 0) {
        this.noise ??= noiseTile()
        const pattern = ctx.createPattern(this.noise, 'repeat')
        if (pattern) {
          const shift = Math.floor(time * 24)
          pattern.setTransform(new DOMMatrix().translate((shift * 37) % 128, (shift * 71) % 128))
          ctx.globalAlpha = layer.opacity * layer.grain * 1.6
          ctx.globalCompositeOperation = 'overlay'
          ctx.fillStyle = pattern
          ctx.fillRect(-w / 2, -h / 2, w, h)
        }
      }
      if (layer.scanlines) {
        ctx.globalAlpha = layer.opacity * 0.15
        ctx.globalCompositeOperation = 'source-over'
        ctx.fillStyle = '#000'
        const step = Math.max(2, 4 * scale)
        for (let y = -h / 2; y < h / 2; y += step) ctx.fillRect(-w / 2, y, w, Math.max(1, scale))
      }
    }
    ctx.restore()
  }

  destroy() {
    this.colorFilters.destroy()
  }
}
