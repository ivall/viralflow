import { fadeGain, itemActiveAt, mediaTimeAt } from '../engine/scene'
import type { MediaAsset, Project, TimelineItem, Track } from '../types'
import { fileUrl } from '@/services/ipc'

/**
 * Owns the <video>, <audio> and <img> elements of the preview and keeps them in
 * sync with the timeline clock. Every clip gets its own element so overlapping
 * clips (transitions, picture-in-picture) of the same file work.
 */

const DRIFT_PLAYING = 0.15
const PRELOAD_AHEAD = 1.5
const MAX_IDLE = 10

interface Voice {
  element: HTMLMediaElement
  gain?: GainNode
  itemId: string
  src: string
  lastUsed: number
  seeking: boolean
  pendingTime: number | null
}

export class MediaPool {
  private videos = new Map<string, Voice>()
  private audios = new Map<string, Voice>()
  private images = new Map<string, HTMLImageElement>()
  private context: AudioContext | null = null
  private container: HTMLElement
  private frameCounter = 0
  master = 1
  onFrameReady: () => void = () => {}

  constructor() {
    this.container = document.createElement('div')
    this.container.setAttribute('aria-hidden', 'true')
    this.container.style.cssText = 'position:fixed;left:-10px;top:-10px;width:1px;height:1px;overflow:hidden;opacity:0;pointer-events:none'
    document.body.appendChild(this.container)
  }

  /** Must be called from a user gesture before playback so audio can start. */
  unlockAudio() {
    if (!this.context) {
      try {
        this.context = new AudioContext({ latencyHint: 'interactive' })
      } catch {
        this.context = null
      }
    }
    if (this.context?.state === 'suspended') void this.context.resume()
  }

  private sourceUrl(asset: MediaAsset) {
    return fileUrl(asset.previewPath ?? asset.path)
  }

  private create(kind: 'video' | 'audio', itemId: string, src: string): Voice {
    const element = document.createElement(kind)
    element.crossOrigin = 'anonymous'
    element.preload = 'auto'
    element.src = src
    if (element instanceof HTMLVideoElement) {
      element.playsInline = true
      element.disablePictureInPicture = true
    }
    const voice: Voice = { element, itemId, src, lastUsed: this.frameCounter, seeking: false, pendingTime: null }
    element.addEventListener('seeked', () => {
      voice.seeking = false
      if (voice.pendingTime !== null) {
        const time = voice.pendingTime
        voice.pendingTime = null
        this.seekVoice(voice, time)
      } else this.onFrameReady()
    })
    element.addEventListener('loadeddata', () => this.onFrameReady())
    this.container.appendChild(element)
    return voice
  }

  private connect(voice: Voice) {
    if (voice.gain || !this.context) return
    try {
      const source = this.context.createMediaElementSource(voice.element)
      voice.gain = this.context.createGain()
      source.connect(voice.gain).connect(this.context.destination)
      voice.element.volume = 1
    } catch {
      // Already connected or not allowed: fall back to element volume.
    }
  }

  private setGain(voice: Voice, value: number) {
    const gain = Math.max(0, value * this.master)
    if (voice.gain && this.context) {
      voice.gain.gain.setTargetAtTime(gain, this.context.currentTime, 0.01)
      voice.element.muted = false
    } else {
      voice.element.muted = gain <= 0
      voice.element.volume = Math.min(1, gain)
    }
  }

  private seekVoice(voice: Voice, time: number) {
    if (voice.seeking) {
      voice.pendingTime = time
      return
    }
    if (Math.abs(voice.element.currentTime - time) < 0.001) return
    voice.seeking = true
    voice.element.currentTime = time
  }

  private syncVoice(voice: Voice, mediaTime: number, rate: number, shouldPlay: boolean, gain: number, fps: number) {
    const element = voice.element
    voice.lastUsed = this.frameCounter
    if (Math.abs(element.playbackRate - rate) > 1e-3) element.playbackRate = rate
    if (shouldPlay) {
      if (this.context && !voice.gain) this.connect(voice)
      if (Math.abs(element.currentTime - mediaTime) > DRIFT_PLAYING * Math.max(1, rate)) this.seekVoice(voice, mediaTime)
      if (element.paused) void element.play().catch(() => undefined)
    } else {
      if (!element.paused) element.pause()
      if (Math.abs(element.currentTime - mediaTime) > 0.5 / fps) this.seekVoice(voice, mediaTime)
    }
    this.setGain(voice, gain)
  }

  video(itemId: string): HTMLVideoElement | null {
    const element = this.videos.get(itemId)?.element as HTMLVideoElement | undefined
    return element && element.readyState >= 2 ? element : null
  }

  image(asset: MediaAsset): HTMLImageElement | null {
    let image = this.images.get(asset.id)
    const src = this.sourceUrl(asset)
    if (!image || image.dataset.src !== src) {
      image = new Image()
      image.crossOrigin = 'anonymous'
      image.decoding = 'async'
      image.dataset.src = src
      image.onload = () => this.onFrameReady()
      image.src = src
      this.images.set(asset.id, image)
    }
    return image.complete && image.naturalWidth > 0 ? image : null
  }

  /** Synchronises all media with the timeline. `visibleVideos` are the video clips drawn this frame. */
  sync(project: Project, assets: Map<string, MediaAsset>, time: number, playing: boolean, visibleVideos: Set<string>) {
    this.frameCounter++
    const wanted = new Set<string>()
    const audioWanted = new Set<string>()
    const tracks: Track[] = project.tracks
    for (const track of tracks) {
      for (const item of track.items) {
        if (item.hidden || !item.assetId) continue
        const asset = assets.get(item.assetId)
        if (!asset) continue
        const near = time >= item.start - PRELOAD_AHEAD && time < item.start + item.duration + 0.5
        if (item.kind === 'video' && (visibleVideos.has(item.id) || near)) {
          wanted.add(item.id)
          this.syncVideo(project, track, item, asset, time, playing, visibleVideos.has(item.id))
        } else if (item.kind === 'audio' && near) {
          audioWanted.add(item.id)
          this.syncAudio(project, track, item, asset, time, playing)
        }
      }
    }
    this.retire(this.videos, wanted)
    this.retire(this.audios, audioWanted)
  }

  private itemGain(track: Track, item: TimelineItem, time: number) {
    if (track.muted || item.muted || !itemActiveAt(item, time)) return 0
    return (item.volume ?? 1) * fadeGain(item, time)
  }

  private syncVideo(project: Project, track: Track, item: TimelineItem, asset: MediaAsset, time: number, playing: boolean, visible: boolean) {
    const src = this.sourceUrl(asset)
    let voice = this.videos.get(item.id)
    if (!voice || voice.src !== src) {
      if (voice) this.dispose(voice)
      voice = this.create('video', item.id, src)
      this.videos.set(item.id, voice)
    }
    const active = itemActiveAt(item, time)
    const mediaTime = mediaTimeAt(item, Math.max(time, item.start), project.fps, asset.duration)
    const gain = asset.hasAudio === false ? 0 : this.itemGain(track, item, time)
    this.syncVoice(voice, mediaTime, item.speed || 1, playing && active && visible, gain, project.fps)
  }

  private syncAudio(project: Project, track: Track, item: TimelineItem, asset: MediaAsset, time: number, playing: boolean) {
    const src = this.sourceUrl(asset)
    let voice = this.audios.get(item.id)
    if (!voice || voice.src !== src) {
      if (voice) this.dispose(voice)
      voice = this.create('audio', item.id, src)
      this.audios.set(item.id, voice)
    }
    const active = itemActiveAt(item, time)
    const mediaTime = mediaTimeAt(item, Math.max(time, item.start), project.fps, asset.duration)
    this.syncVoice(voice, mediaTime, item.speed || 1, playing && active, this.itemGain(track, item, time), project.fps)
  }

  private retire(map: Map<string, Voice>, wanted: Set<string>) {
    for (const [id, voice] of map) {
      if (wanted.has(id)) continue
      if (!voice.element.paused) voice.element.pause()
      if (this.frameCounter - voice.lastUsed > 60 * MAX_IDLE || map.size > 16) {
        this.dispose(voice)
        map.delete(id)
      }
    }
  }

  private dispose(voice: Voice) {
    voice.element.pause()
    voice.gain?.disconnect()
    voice.element.removeAttribute('src')
    voice.element.load()
    voice.element.remove()
  }

  pauseAll() {
    for (const voice of [...this.videos.values(), ...this.audios.values()]) voice.element.pause()
  }

  destroy() {
    for (const voice of [...this.videos.values(), ...this.audios.values()]) this.dispose(voice)
    this.videos.clear()
    this.audios.clear()
    this.images.clear()
    void this.context?.close().catch(() => undefined)
    this.container.remove()
  }
}
