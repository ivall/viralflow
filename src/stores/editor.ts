import { acceptHMRUpdate, defineStore } from 'pinia'
import { computed, ref, shallowRef, toRaw, watch } from 'vue'
import { defaultInDuration, defaultOutDuration } from '@/lib/engine/animation'
import { adjacentKeyframe, normalizeTransform, patchKeyframeAt, retimeKeyframes, setKeyframeEasing, sortedKeyframes, toggleKeyframe, trimKeyframes } from '@/lib/engine/keyframes'
import { clamp, uid } from '@/lib/engine/math'
import { contentEnd } from '@/lib/engine/scene'
import { DEFAULT_TEXT_STYLE } from '@/lib/engine/text'
import { DEFAULT_TRANSITION, repairTransitions, transitionDuration } from '@/lib/engine/transitions'
import { effectWindow, MIN_EFFECT } from '@/lib/engine/effects'
import { migrateProject } from '@/lib/migrate'
import { CAPTION_PRESET, createTrack, mediaItemDefaults, presetTextStyle, trackKindFor, type TextPreset } from '@/lib/project'
import { MIN_CLIP_DURATION, placeTimelineSelection, placeTimelineSpan, repairTimelineLayout, rippleTimelineResize, trimTimelineSpan, type TimelineMoveMode } from '@/lib/timeline/placement'
import type { AnimationInKind, AnimationOutKind, CaptionCue, EffectKind, EffectRange, Easing, ExportSettings, ItemKind, MediaAsset, Project, TextStyle, TimelineItem, Track, TrackKind, Transform, TransitionKind } from '@/lib/types'
import { errorMessage, ipc } from '@/services/ipc'
import { createPreview } from '@/services/media'
import { useSettings } from './settings'
import { useToasts } from './toast'

/** Upper bound used by placement helpers; the timeline itself is open-ended. */
const HORIZON = 24 * 3600
const HISTORY_LIMIT = 120

type SaveState = 'saved' | 'saving' | 'dirty' | 'error'

let clipboard: TimelineItem[] = []

export interface ItemPlacement {
  trackId?: string
  start?: number
  /** Always create a fresh track (used when dropping between tracks). */
  newTrackIndex?: number
}

export const useEditor = defineStore('editor', () => {
  const project = ref<Project | null>(null)
  const currentTime = ref(0)
  const playing = ref(false)
  const selectedIds = ref<string[]>([])
  const primaryId = ref<string | null>(null)
  const zoom = ref(60)
  const snapping = ref(true)
  const past = shallowRef<string[]>([])
  const future = shallowRef<string[]>([])
  const saveState = ref<SaveState>('saved')
  const revision = ref(0)
  /** Assets whose file is missing on disk. */
  const offline = ref<Set<string>>(new Set())
  /** Provided by the player: returns a JPEG data URL of the current frame. */
  const thumbnailProvider = shallowRef<(() => Promise<string | null>) | null>(null)

  let saveTimer: ReturnType<typeof setTimeout> | undefined
  let saving: Promise<void> | null = null
  let saveAgain = false
  let lastNudge = 0
  let frameHandle = 0
  let clockOrigin = 0
  let clockStart = 0

  // ------------------------------------------------------------ derived

  const tracks = computed(() => project.value?.tracks ?? [])
  const fps = computed(() => project.value?.fps ?? 30)
  const frame = computed(() => 1 / fps.value)
  const assetsById = computed(() => new Map((project.value?.assets ?? []).map(asset => [asset.id, asset])))
  const playbackEnd = computed(() => project.value ? contentEnd(project.value) : 0)
  const timelineLength = computed(() => Math.max(30, playbackEnd.value * 1.25 + 10))
  const itemIndex = computed(() => {
    const map = new Map<string, { item: TimelineItem; track: Track; trackIndex: number }>()
    tracks.value.forEach((track, trackIndex) => track.items.forEach(item => map.set(item.id, { item, track, trackIndex })))
    return map
  })
  const selectedItems = computed(() => selectedIds.value.map(id => itemIndex.value.get(id)?.item).filter((item): item is TimelineItem => Boolean(item)))
  const selectedItem = computed(() => (primaryId.value && itemIndex.value.get(primaryId.value)?.item) || null)
  const selectedTrack = computed(() => (primaryId.value && itemIndex.value.get(primaryId.value)?.track) || null)
  const canUndo = computed(() => past.value.length > 0)
  const canRedo = computed(() => future.value.length > 0)
  const keyframeTolerance = computed(() => 0.5 / fps.value)

  function find(id: string) {
    return itemIndex.value.get(id) ?? null
  }

  function editable(id: string) {
    const found = find(id)
    return found && !found.item.locked && !found.track.locked ? found : null
  }

  // ------------------------------------------------------------ lifecycle

  async function open(id: string) {
    stop()
    const document = migrateProject(await ipc.loadProject(id))
    project.value = document
    currentTime.value = 0
    selectedIds.value = []
    primaryId.value = null
    past.value = []
    future.value = []
    saveState.value = 'saved'
    revision.value = 0
    snapping.value = useSettings().value.snap
    tracks.value.forEach(track => repairTrack(track))
    const paths = document.assets.flatMap(asset => [asset.path, asset.previewPath].filter((path): path is string => Boolean(path)))
    if (paths.length) await ipc.allowMedia(paths)
    await checkMedia()
    const pending = document.assets.filter(asset => asset.needsProxy && !asset.previewPath && !offline.value.has(asset.id))
    pending.forEach(asset => void createPreview(asset))
    const fit = Math.max(10, playbackEnd.value)
    zoom.value = clamp(900 / fit, 8, 160)
  }

  async function checkMedia() {
    const assets = project.value?.assets ?? []
    if (!assets.length) {
      offline.value = new Set()
      return
    }
    try {
      const present = await ipc.checkPaths(assets.map(asset => asset.path))
      offline.value = new Set(assets.filter((_, index) => !present[index]).map(asset => asset.id))
    } catch {
      offline.value = new Set()
    }
  }

  async function close() {
    stop()
    if (!project.value) return
    await flushSave()
    await saveThumbnail()
    project.value = null
  }

  async function saveThumbnail() {
    if (!project.value || !thumbnailProvider.value) return
    try {
      const dataUrl = await thumbnailProvider.value()
      if (dataUrl) await ipc.saveProjectThumbnail(project.value.id, dataUrl.split(',')[1])
    } catch {
      // Thumbnails are cosmetic; a failure must never block leaving the editor.
    }
  }

  // ------------------------------------------------------------ history & saving

  function snapshot() {
    if (!project.value) return
    past.value = [...past.value.slice(-(HISTORY_LIMIT - 1)), JSON.stringify(toRaw(project.value))]
    future.value = []
    lastNudge = 0
  }

  /** Call before a continuous change (drag, slider) so the whole gesture is one undo step. */
  function beginChange() {
    snapshot()
  }

  function touched() {
    if (!project.value) return
    project.value.updatedAt = new Date().toISOString()
    revision.value++
  }

  function restore(serialized: string) {
    project.value = migrateProject(JSON.parse(serialized))
    selectMany(selectedIds.value)
    touched()
  }

  function undo() {
    if (!project.value || !past.value.length) return
    const previous = past.value[past.value.length - 1]
    past.value = past.value.slice(0, -1)
    future.value = [...future.value, JSON.stringify(toRaw(project.value))]
    restore(previous)
  }

  function redo() {
    if (!project.value || !future.value.length) return
    const next = future.value[future.value.length - 1]
    future.value = future.value.slice(0, -1)
    past.value = [...past.value, JSON.stringify(toRaw(project.value))]
    restore(next)
  }

  async function save() {
    if (!project.value) return
    if (saving) {
      saveAgain = true
      return saving
    }
    saving = (async () => {
      do {
        saveAgain = false
        saveState.value = 'saving'
        try {
          await ipc.saveProject(JSON.parse(JSON.stringify(toRaw(project.value))))
          if (!saveAgain) saveState.value = 'saved'
        } catch (error) {
          saveState.value = 'error'
          useToasts().error('Project could not be saved', errorMessage(error))
          break
        }
      } while (saveAgain && project.value)
    })().finally(() => { saving = null })
    return saving
  }

  async function flushSave() {
    if (saveTimer) {
      clearTimeout(saveTimer)
      saveTimer = undefined
    }
    if (saveState.value !== 'saved') await save()
    if (saving) await saving
  }

  watch(revision, () => {
    if (!project.value) return
    saveState.value = 'dirty'
    if (!useSettings().value.autosave) return
    if (saveTimer) clearTimeout(saveTimer)
    saveTimer = setTimeout(() => { saveTimer = undefined; void save() }, 700)
  })

  // ------------------------------------------------------------ playback clock

  function tick(now: number) {
    if (!playing.value) return
    const time = clockStart + (now - clockOrigin) / 1000
    if (time >= playbackEnd.value) {
      currentTime.value = playbackEnd.value
      stop()
      return
    }
    currentTime.value = time
    frameHandle = requestAnimationFrame(tick)
  }

  function play() {
    if (playing.value || playbackEnd.value <= 0) return
    if (currentTime.value >= playbackEnd.value - frame.value) currentTime.value = 0
    playing.value = true
    clockOrigin = performance.now()
    clockStart = currentTime.value
    frameHandle = requestAnimationFrame(tick)
  }

  function stop() {
    playing.value = false
    cancelAnimationFrame(frameHandle)
    currentTime.value = Math.round(currentTime.value * fps.value) / fps.value
  }

  function togglePlay() {
    if (playing.value) stop()
    else play()
  }

  function seek(time: number) {
    const value = clamp(time, 0, Math.max(timelineLength.value, playbackEnd.value))
    if (playing.value) {
      clockOrigin = performance.now()
      clockStart = value
      currentTime.value = value
    } else currentTime.value = Math.round(value * fps.value) / fps.value
  }

  function stepFrames(count: number) {
    if (playing.value) stop()
    seek(currentTime.value + count * frame.value)
  }

  // ------------------------------------------------------------ selection

  function select(id: string | null, additive = false) {
    if (!id) {
      selectedIds.value = []
      primaryId.value = null
      return
    }
    if (additive) {
      selectedIds.value = selectedIds.value.includes(id) ? selectedIds.value.filter(value => value !== id) : [...selectedIds.value, id]
      primaryId.value = selectedIds.value.includes(id) ? id : selectedIds.value.at(-1) ?? null
      return
    }
    // Captions share a group id for bulk styling, but a click still selects one caption.
    const group = find(id)?.item.groupId
    const linked = group && !group.startsWith('captions-')
    selectedIds.value = linked ? tracks.value.flatMap(track => track.items).filter(item => item.groupId === group).map(item => item.id) : [id]
    primaryId.value = id
  }

  function selectMany(ids: string[]) {
    const existing = new Set(itemIndex.value.keys())
    selectedIds.value = [...new Set(ids)].filter(id => existing.has(id))
    primaryId.value = selectedIds.value.includes(primaryId.value ?? '') ? primaryId.value : selectedIds.value.at(-1) ?? null
  }

  function selectAll() {
    selectMany(tracks.value.flatMap(track => track.items.map(item => item.id)))
  }

  // ------------------------------------------------------------ track helpers

  function repairTrack(track: Track) {
    const layout = repairTimelineLayout(track.items)
    for (const item of track.items) {
      item.start = layout.starts.get(item.id) ?? item.start
      item.duration = Math.max(MIN_CLIP_DURATION, item.duration)
      if (item.keyframes?.length) item.keyframes = sortedKeyframes(item)
    }
    track.items.sort((a, b) => a.start - b.start || a.id.localeCompare(b.id))
    repairTransitions(track, fps.value)
  }

  function repairAll(...changed: Track[]) {
    for (const track of new Set(changed)) {
      track.items.sort((a, b) => a.start - b.start || a.id.localeCompare(b.id))
      repairTransitions(track, fps.value)
    }
  }

  function insertTrack(kind: TrackKind, near?: string): Track {
    const list = project.value!.tracks
    const track = createTrack(kind, list)
    const nearIndex = near ? list.findIndex(candidate => candidate.id === near) : -1
    if (kind === 'audio') list.push(track)
    else if (kind === 'text') list.unshift(track)
    else if (nearIndex >= 0) list.splice(nearIndex, 0, track)
    else {
      const firstVideo = list.findIndex(candidate => candidate.kind === 'video')
      list.splice(firstVideo >= 0 ? firstVideo : list.findIndex(candidate => candidate.kind === 'audio') >= 0 ? list.findIndex(candidate => candidate.kind === 'audio') : list.length, 0, track)
    }
    return track
  }

  /** Finds a track with room for the span, creating one when needed. */
  function placeSpan(kind: ItemKind, span: { id: string; start: number; duration: number }, preferredTrackId?: string, exact = false) {
    const trackKind = trackKindFor(kind)
    const list = project.value!.tracks
    const preferred = preferredTrackId ? list.find(track => track.id === preferredTrackId && track.kind === trackKind && !track.locked) : undefined
    const candidates = [preferred, ...list.filter(track => track.kind === trackKind && !track.locked && track !== preferred).reverse()].filter(Boolean) as Track[]
    for (const track of candidates) {
      const placement = placeTimelineSpan(track.items, span, HORIZON)
      if (placement && (!exact || Math.abs(placement.start - span.start) < 1e-6)) return { track, start: placement.start }
      if (track === preferred && placement && !exact) return { track, start: placement.start }
    }
    const track = insertTrack(trackKind, preferred?.id ?? preferredTrackId)
    return { track, start: Math.max(0, span.start) }
  }

  // ------------------------------------------------------------ adding

  function addItem(defaults: Partial<TimelineItem> & { kind: ItemKind }, placement: ItemPlacement = {}) {
    if (!project.value) return null
    snapshot()
    const id = uid(defaults.kind)
    const start = Math.max(0, placement.start ?? currentTime.value)
    const duration = Math.max(MIN_CLIP_DURATION, defaults.duration ?? 3)
    let target: { track: Track; start: number }
    if (placement.newTrackIndex !== undefined) {
      const track = createTrack(trackKindFor(defaults.kind), project.value.tracks)
      project.value.tracks.splice(clamp(placement.newTrackIndex, 0, project.value.tracks.length), 0, track)
      target = { track, start }
    } else target = placeSpan(defaults.kind, { id, start, duration }, placement.trackId, placement.trackId === undefined)
    const item: TimelineItem = { name: defaults.kind, ...structuredClone(defaults), id, trackId: target.track.id, start: target.start, duration }
    target.track.items.push(item)
    repairAll(target.track)
    select(item.id)
    touched()
    return item
  }

  function addAsset(asset: MediaAsset, placement: ItemPlacement = {}) {
    if (!project.value) return null
    return addItem(mediaItemDefaults(asset, project.value), placement)
  }

  /** Adds several assets one after another (e.g. a multi-file drop); audio starts with the visuals. */
  function addAssetsSequence(assets: MediaAsset[], placement: ItemPlacement = {}) {
    const start = placement.start ?? currentTime.value
    const cursors: Record<'visual' | 'audio', { time: number; trackId?: string; newTrackIndex?: number }> = {
      visual: { time: start, trackId: placement.trackId, newTrackIndex: placement.newTrackIndex },
      audio: { time: start }
    }
    const created: string[] = []
    const before = past.value.length
    for (const asset of assets) {
      const cursor = cursors[asset.kind === 'audio' ? 'audio' : 'visual']
      const item = addAsset(asset, { trackId: cursor.trackId, start: cursor.time, newTrackIndex: cursor.newTrackIndex })
      if (!item) continue
      created.push(item.id)
      cursor.time = item.start + item.duration
      cursor.trackId = item.trackId
      cursor.newTrackIndex = undefined
    }
    // One undo step for the whole drop.
    if (past.value.length > before + 1) past.value = past.value.slice(0, before + 1)
    selectMany(created)
  }

  function registerAssets(assets: MediaAsset[]) {
    if (!project.value || !assets.length) return
    const known = new Set(project.value.assets.map(asset => asset.path))
    const fresh = assets.filter(asset => !known.has(asset.path))
    if (!fresh.length) return
    project.value.assets.push(...fresh)
    touched()
  }

  function updateAsset(id: string, patch: Partial<MediaAsset>) {
    const asset = project.value?.assets.find(candidate => candidate.id === id)
    if (!asset) return
    Object.assign(asset, patch)
    if (patch.path && offline.value.has(id)) {
      const next = new Set(offline.value)
      next.delete(id)
      offline.value = next
    }
    touched()
  }

  function removeAsset(id: string) {
    if (!project.value) return
    snapshot()
    project.value.assets = project.value.assets.filter(asset => asset.id !== id)
    for (const track of project.value.tracks) {
      track.items = track.items.filter(item => item.assetId !== id)
      repairTransitions(track, fps.value)
    }
    selectMany(selectedIds.value)
    touched()
  }

  // ------------------------------------------------------------ updating

  function applyPatch(item: TimelineItem, track: Track, patch: Partial<TimelineItem>) {
    const next = { ...patch }
    if (next.speed !== undefined) {
      setSpeedOf(item, track, next.speed)
      delete next.speed
    }
    if (next.start !== undefined || next.duration !== undefined) {
      const candidate = { id: item.id, start: Math.max(0, next.start ?? item.start), duration: Math.max(MIN_CLIP_DURATION, next.duration ?? item.duration) }
      const before = { ...item }
      const placement = next.start !== undefined
        ? placeTimelineSpan(track.items, candidate, HORIZON)
        : trimTimelineSpan(track.items, item, candidate, 'end', HORIZON)
      if (placement) {
        item.start = placement.start
        item.duration = placement.duration
        if (before.keyframes?.length && item.duration < before.duration - 1e-6) item.keyframes = trimKeyframes(before, 0, item.duration)
      }
      delete next.start
      delete next.duration
    }
    if (next.text) next.text = { ...DEFAULT_TEXT_STYLE, ...item.text, ...next.text }
    if (next.shape && item.shape) next.shape = { ...item.shape, ...next.shape }
    if (next.adjust) next.adjust = { ...{ exposure: 0, brightness: 0, contrast: 0, saturation: 0, temperature: 0, tint: 0, hue: 0, vignette: 0 }, ...item.adjust, ...next.adjust }
    Object.assign(item, next)
    if (item.text) item.name = item.text.content.split('\n')[0].slice(0, 40) || item.name
  }

  /** Patches every editable selected item. `record` = create an undo step. */
  function updateSelected(patch: Partial<TimelineItem>, record = true) {
    const targets = selectedIds.value.map(editable).filter(Boolean) as Array<{ item: TimelineItem; track: Track }>
    if (!targets.length) return
    if (record) snapshot()
    for (const { item, track } of targets) applyPatch(item, track, patch)
    repairAll(...targets.map(target => target.track))
    touched()
  }

  function updateItem(id: string, patch: Partial<TimelineItem>, record = true) {
    const found = editable(id)
    if (!found) return
    if (record) snapshot()
    applyPatch(found.item, found.track, patch)
    repairAll(found.track)
    touched()
  }

  function updateText(patch: Partial<TextStyle>, record = true) {
    const targets = selectedItems.value.filter(item => item.kind === 'text')
    if (!targets.length) return
    if (record) snapshot()
    // Style changes go to every selected text; the words only to the one being edited.
    const { content, ...style } = patch
    for (const item of targets) {
      if (!editable(item.id)) continue
      const own = content !== undefined && item.id === selectedItem.value?.id
      item.text = { ...DEFAULT_TEXT_STYLE, ...item.text, ...style, ...(own ? { content } : {}) }
      if (own) item.name = content.split('\n')[0].slice(0, 40) || 'Text'
    }
    touched()
  }

  /** Gives every selected text the preset's look; each keeps its own words and layout. */
  function applyTextPreset(preset: TextPreset) {
    const targets = selectedItems.value.filter(item => item.kind === 'text' && editable(item.id))
    if (!targets.length) return
    snapshot()
    for (const item of targets) {
      const continued = !item.animIn && (item.text?.highlightIndex ?? -1) > 0 && Boolean(item.groupId?.startsWith('captions-'))
      item.text = presetTextStyle({ ...DEFAULT_TEXT_STYLE, ...item.text }, preset)
      item.animIn = preset.animIn && !continued ? { ...preset.animIn } : undefined
    }
    touched()
  }

  /** Transform edits write into a keyframe when the clip is animated with keyframes. */
  function setTransform(updates: Array<{ id: string; transform: Partial<Transform> }>, record = false) {
    if (record) snapshot()
    for (const update of updates) {
      const found = editable(update.id)
      if (!found) continue
      const { item } = found
      const elapsed = currentTime.value - item.start
      const inside = elapsed >= -keyframeTolerance.value && elapsed <= item.duration + keyframeTolerance.value
      if (item.keyframes?.length && inside) item.keyframes = patchKeyframeAt(item, elapsed, update.transform, keyframeTolerance.value)
      else item.transform = normalizeTransform({ ...normalizeTransform(item.transform), ...update.transform })
    }
    touched()
  }

  function setSpeedOf(item: TimelineItem, track: Track, value: number) {
    const speed = clamp(Number.isFinite(value) ? value : 1, 0.1, 10)
    const current = item.speed || 1
    if (Math.abs(speed - current) < 1e-6) return
    const before = item.duration
    const sourceLength = item.duration * current
    const layout = rippleTimelineResize(track.items, item.id, sourceLength / speed)
    item.speed = speed
    item.duration = layout.duration
    if (item.keyframes?.length) item.keyframes = retimeKeyframes(item.keyframes, before, item.duration)
    if (item.effectRange && before > 0) {
      const ratio = item.duration / before
      item.effectRange = { start: item.effectRange.start * ratio, duration: item.effectRange.duration * ratio }
    }
    for (const neighbour of track.items) {
      const start = layout.starts.get(neighbour.id)
      if (start !== undefined) neighbour.start = start
    }
  }

  // ------------------------------------------------------------ moving & trimming

  function moveItem(id: string, start: number, targetTrackId?: string) {
    const found = editable(id)
    if (!found) return false
    const { item, track } = found
    const target = targetTrackId && targetTrackId !== track.id
      ? project.value!.tracks.find(candidate => candidate.id === targetTrackId && candidate.kind === track.kind && !candidate.locked)
      : track
    if (!target) return false
    const placement = placeTimelineSpan(target.items, { id, start, duration: item.duration }, HORIZON, target === track ? [id] : [])
    if (!placement) return false
    item.start = placement.start
    if (target !== track) {
      track.items = track.items.filter(candidate => candidate.id !== id)
      item.trackId = target.id
      target.items.push(item)
    }
    repairAll(track, target)
    touched()
    return true
  }

  function moveItems(moves: Array<{ id: string; start: number }>) {
    const entries = moves.map(move => ({ move, found: editable(move.id) })).filter(entry => entry.found) as Array<{ move: { id: string; start: number }; found: { item: TimelineItem; track: Track } }>
    const placements = placeTimelineSelection(
      tracks.value.map(track => ({ id: track.id, items: track.items })),
      entries.map(({ move, found }) => ({ id: move.id, trackId: found.track.id, start: move.start, duration: found.item.duration })),
      HORIZON
    )
    if (!placements) return
    for (const placement of placements) {
      const entry = entries.find(candidate => candidate.move.id === placement.id)
      if (entry) entry.found.item.start = placement.start
    }
    repairAll(...entries.map(entry => entry.found.track))
    touched()
  }

  /** Trims one edge. `edgeTime` is the requested timeline position of that edge. */
  function trimItem(id: string, edge: 'start' | 'end', edgeTime: number, original: { start: number; duration: number; sourceIn: number; effectRange?: EffectRange }) {
    const found = editable(id)
    if (!found) return
    const { item, track } = found
    const speed = item.speed || 1
    const isMedia = item.kind === 'video' || item.kind === 'audio'
    const asset = item.assetId ? assetsById.value.get(item.assetId) : undefined
    const sourceDuration = asset?.duration ?? Infinity
    const before = { ...item, start: original.start, duration: original.duration }
    if (edge === 'start') {
      const minStart = isMedia ? original.start - original.sourceIn / speed : 0
      const requested = clamp(edgeTime, Math.max(0, minStart), original.start + original.duration - MIN_CLIP_DURATION)
      const placement = trimTimelineSpan(track.items, { id, start: original.start, duration: original.duration }, { id, start: requested, duration: original.start + original.duration - requested }, 'start', HORIZON)
      item.start = placement.start
      item.duration = placement.duration
      if (isMedia || item.kind === 'image') item.sourceIn = Math.max(0, original.sourceIn + (placement.start - original.start) * speed)
      if (before.keyframes?.length) item.keyframes = trimKeyframes(before, Math.max(0, placement.start - original.start), placement.duration)
    } else {
      const maxEnd = isMedia && item.kind !== 'image' ? original.start + (sourceDuration - original.sourceIn) / speed : Infinity
      const requested = clamp(edgeTime, original.start + MIN_CLIP_DURATION, maxEnd)
      const placement = trimTimelineSpan(track.items, { id, start: original.start, duration: original.duration }, { id, start: original.start, duration: requested - original.start }, 'end', HORIZON)
      item.duration = placement.duration
      if (before.keyframes?.length && placement.duration < original.duration) item.keyframes = trimKeyframes(before, 0, placement.duration)
    }
    if (original.effectRange) {
      // Ranged effects stay where they are on the timeline while the clip edge moves.
      const shift = item.start - original.start
      item.effectRange = effectWindow({ duration: item.duration, effectRange: { start: original.effectRange.start - shift, duration: original.effectRange.duration + Math.min(0, original.effectRange.start - shift) } })
    }
    repairAll(track)
    touched()
  }

  function nudgeSelected(delta: number) {
    const entries = selectedIds.value.map(editable).filter(Boolean) as Array<{ item: TimelineItem; track: Track }>
    if (!entries.length) return
    const now = Date.now()
    if (now - lastNudge > 500) snapshot()
    lastNudge = now
    moveItems(entries.map(({ item }) => ({ id: item.id, start: Math.max(0, item.start + delta) })))
  }

  function trimToPlayhead(edge: 'start' | 'end') {
    const target = selectedItem.value
    const found = target ? editable(target.id) : null
    if (!found) return
    const { item } = found
    const time = currentTime.value
    if (time <= item.start + MIN_CLIP_DURATION || time >= item.start + item.duration - MIN_CLIP_DURATION) return
    snapshot()
    trimItem(item.id, edge, time, { start: item.start, duration: item.duration, sourceIn: item.sourceIn ?? 0, effectRange: item.effectRange && { ...item.effectRange } })
  }

  // ------------------------------------------------------------ structural edits

  function splitAt(time = currentTime.value) {
    const candidates = selectedIds.value.length
      ? selectedIds.value.map(editable).filter(Boolean) as Array<{ item: TimelineItem; track: Track }>
      : tracks.value.filter(track => !track.locked && !track.hidden).flatMap(track => track.items.filter(item => !item.locked).map(item => ({ item, track })))
    const targets = candidates.filter(({ item }) => time > item.start + MIN_CLIP_DURATION && time < item.start + item.duration - MIN_CLIP_DURATION)
    if (!targets.length) return
    snapshot()
    const created: string[] = []
    for (const { item, track } of targets) {
      const before = { ...item }
      const offset = time - item.start
      const right: TimelineItem = structuredClone(toRaw(item))
      right.id = uid(item.kind)
      right.start = time
      right.duration = before.duration - offset
      right.sourceIn = (item.sourceIn ?? 0) + offset * (item.speed || 1)
      right.keyframes = before.keyframes?.length ? trimKeyframes(before, offset, right.duration) : undefined
      delete right.transition
      if (right.animIn) right.animIn = { kind: 'none', duration: 0 }
      if (item.animOut) item.animOut = { kind: 'none', duration: 0 }
      if (item.fadeOut) { right.fadeOut = item.fadeOut; item.fadeOut = 0 }
      if (right.fadeIn) right.fadeIn = 0
      if (before.effect && before.effectRange) {
        const window = effectWindow(before)
        const leftEnd = Math.min(offset, window.start + window.duration)
        if (leftEnd - window.start >= MIN_EFFECT) item.effectRange = { start: window.start, duration: leftEnd - window.start }
        else { delete item.effect; delete item.effectRange }
        const rightStart = Math.max(0, window.start - offset)
        const rightEnd = window.start + window.duration - offset
        if (rightEnd - rightStart >= MIN_EFFECT) right.effectRange = { start: rightStart, duration: rightEnd - rightStart }
        else { delete right.effect; delete right.effectRange }
      }
      item.duration = offset
      item.keyframes = before.keyframes?.length ? trimKeyframes(before, 0, offset) : undefined
      track.items.push(right)
      created.push(right.id)
      repairAll(track)
    }
    selectMany(created)
    touched()
  }

  function deleteSelected(ripple = false) {
    const targets = selectedIds.value.map(editable).filter(Boolean) as Array<{ item: TimelineItem; track: Track }>
    if (!targets.length) return
    snapshot()
    const ids = new Set(targets.map(target => target.item.id))
    if (ripple) {
      for (const track of new Set(targets.map(target => target.track))) {
        const removed = track.items.filter(item => ids.has(item.id)).sort((a, b) => b.start - a.start)
        track.items = track.items.filter(item => !ids.has(item.id))
        for (const gap of removed) {
          for (const item of track.items) if (item.start >= gap.start + gap.duration - 1e-6) item.start = Math.max(0, item.start - gap.duration)
        }
        repairTrack(track)
      }
    } else {
      for (const track of new Set(targets.map(target => target.track))) {
        track.items = track.items.filter(item => !ids.has(item.id))
        repairAll(track)
      }
    }
    select(null)
    touched()
  }

  function cloneItems(sources: TimelineItem[], offset: (item: TimelineItem) => number, preferTrack: (item: TimelineItem) => string | undefined) {
    const groups = new Map<string, string>()
    const created: string[] = []
    const changed = new Set<Track>()
    for (const source of sources) {
      const copy: TimelineItem = structuredClone(toRaw(source))
      copy.id = uid(source.kind)
      delete copy.transition
      if (copy.groupId) {
        if (!groups.has(copy.groupId)) groups.set(copy.groupId, uid('group'))
        copy.groupId = groups.get(copy.groupId)
      }
      const target = placeSpan(copy.kind, { id: copy.id, start: offset(source), duration: copy.duration }, preferTrack(source))
      copy.start = target.start
      copy.trackId = target.track.id
      target.track.items.push(copy)
      changed.add(target.track)
      created.push(copy.id)
    }
    repairAll(...changed)
    selectMany(created)
    touched()
  }

  function duplicateSelected() {
    const sources = selectedIds.value.map(editable).filter(Boolean).map(found => found!.item)
    if (!sources.length) return
    snapshot()
    const end = Math.max(...sources.map(item => item.start + item.duration))
    const first = Math.min(...sources.map(item => item.start))
    cloneItems(sources, item => end + (item.start - first), item => item.trackId)
  }

  function copySelected() {
    clipboard = selectedItems.value.map(item => structuredClone(toRaw(item)))
    return clipboard.length
  }

  function cutSelected() {
    if (!copySelected()) return
    deleteSelected()
  }

  function paste() {
    if (!project.value || !clipboard.length) return
    snapshot()
    const first = Math.min(...clipboard.map(item => item.start))
    cloneItems(clipboard, item => currentTime.value + item.start - first, item => item.trackId)
  }

  function groupSelected() {
    if (selectedIds.value.length < 2) return
    snapshot()
    const group = uid('group')
    selectedItems.value.forEach(item => { item.groupId = group })
    touched()
  }

  function ungroupSelected() {
    const grouped = selectedItems.value.filter(item => item.groupId)
    if (!grouped.length) return
    snapshot()
    grouped.forEach(item => { delete item.groupId })
    touched()
  }

  function toggleItemFlag(key: 'locked' | 'hidden' | 'muted') {
    const targets = selectedItems.value.filter(item => key !== 'muted' || item.kind === 'video' || item.kind === 'audio')
    if (!targets.length) return
    snapshot()
    const value = !targets.every(item => item[key])
    targets.forEach(item => { item[key] = value || undefined })
    tracks.value.forEach(track => repairTransitions(track, fps.value))
    touched()
  }

  // ------------------------------------------------------------ keyframes

  function toggleKeyframeAtPlayhead() {
    const item = selectedItem.value
    if (!item || item.kind === 'audio' || !editable(item.id)) return
    const elapsed = currentTime.value - item.start
    if (elapsed < 0 || elapsed > item.duration) return
    snapshot()
    item.keyframes = toggleKeyframe(item, elapsed, keyframeTolerance.value)
    if (!item.keyframes.length) delete item.keyframes
    touched()
  }

  function jumpKeyframe(direction: -1 | 1) {
    const item = selectedItem.value
    if (!item) return
    const keyframe = adjacentKeyframe(item, currentTime.value - item.start, direction, keyframeTolerance.value)
    if (keyframe) seek(item.start + keyframe.time)
  }

  function setKeyframeEasingAtPlayhead(easing: Easing) {
    const item = selectedItem.value
    if (!item?.keyframes?.length || !editable(item.id)) return
    snapshot()
    item.keyframes = setKeyframeEasing(item, currentTime.value - item.start, easing, keyframeTolerance.value)
    touched()
  }

  function moveKeyframe(itemId: string, keyframeId: string, time: number) {
    const found = editable(itemId)
    if (!found?.item.keyframes) return
    const snapped = clamp(Math.round(time * fps.value) / fps.value, 0, found.item.duration)
    found.item.keyframes = found.item.keyframes
      .filter(keyframe => keyframe.id === keyframeId || Math.abs(keyframe.time - snapped) > keyframeTolerance.value)
      .map(keyframe => keyframe.id === keyframeId ? { ...keyframe, time: snapped } : keyframe)
      .sort((a, b) => a.time - b.time)
    touched()
  }

  function clearKeyframes() {
    const item = selectedItem.value
    if (!item?.keyframes?.length || !editable(item.id)) return
    snapshot()
    item.transform = normalizeTransform(item.transform)
    delete item.keyframes
    touched()
  }

  // ------------------------------------------------------------ animations & transitions

  function setAnimation(slot: 'in' | 'out', kind: AnimationInKind | AnimationOutKind) {
    const targets = selectedItems.value.filter(item => item.kind !== 'audio')
    if (!targets.length) return
    snapshot()
    for (const item of targets) {
      if (!editable(item.id)) continue
      if (slot === 'in') item.animIn = { kind: kind as AnimationInKind, duration: item.animIn?.kind !== 'none' && item.animIn?.duration ? item.animIn.duration : defaultInDuration(kind as AnimationInKind) }
      else item.animOut = { kind: kind as AnimationOutKind, duration: item.animOut?.kind !== 'none' && item.animOut?.duration ? item.animOut.duration : defaultOutDuration(kind as AnimationOutKind) }
    }
    touched()
  }

  function setTransition(itemId: string, kind: TransitionKind, duration?: number, record = true) {
    const found = editable(itemId)
    if (!found) return
    if (record) snapshot()
    if (kind === 'none') delete found.item.transition
    else {
      found.item.transition = { kind, duration: duration ?? found.item.transition?.duration ?? DEFAULT_TRANSITION }
      const previous = found.track.items.find(item => Math.abs(item.start + item.duration - found.item.start) < 0.5 / fps.value && item.id !== itemId)
      if (previous) found.item.transition.duration = transitionDuration(found.item, previous)
    }
    repairTransitions(found.track, fps.value)
    touched()
  }

  /** Applies the same transition to every cut of the track. */
  function setTransitionForTrack(trackId: string, kind: TransitionKind, duration: number) {
    const track = tracks.value.find(candidate => candidate.id === trackId)
    if (!track || track.locked) return
    snapshot()
    for (const item of track.items) if (!item.locked) item.transition = { kind, duration }
    repairTransitions(track, fps.value)
    touched()
  }

  // ------------------------------------------------------------ effects

  /** Puts an effect on clips. A new effect covers the whole clip; `range` limits it. */
  function setEffect(ids: string[], kind: EffectKind, range?: EffectRange) {
    const targets = ids.map(editable).filter(found => found && found.item.kind !== 'audio' && found.item.kind !== 'shape') as Array<{ item: TimelineItem; track: Track }>
    if (!targets.length) return
    snapshot()
    for (const { item } of targets) {
      if (kind === 'none') { delete item.effect; delete item.effectRange; continue }
      if (range) item.effectRange = effectWindow({ duration: item.duration, effectRange: range })
      else if (item.effect !== kind) delete item.effectRange
      item.effect = kind
    }
    touched()
  }

  /** Moves or stretches the part of the clip an effect covers (relative to the clip start). */
  function setEffectRange(itemId: string, range: EffectRange, record = false) {
    const found = editable(itemId)
    if (!found?.item.effect) return
    if (record) snapshot()
    const window = effectWindow({ duration: found.item.duration, effectRange: range })
    if (window.start <= 1e-6 && window.duration >= found.item.duration - 1e-6) delete found.item.effectRange
    else found.item.effectRange = window
    touched()
  }

  // ------------------------------------------------------------ tracks

  function addTrack(kind: TrackKind) {
    if (!project.value) return
    snapshot()
    insertTrack(kind)
    touched()
  }

  function updateTrack(trackId: string, patch: Partial<Pick<Track, 'name' | 'locked' | 'muted' | 'hidden'>>) {
    const track = tracks.value.find(candidate => candidate.id === trackId)
    if (!track) return
    snapshot()
    Object.assign(track, patch)
    touched()
  }

  function removeTrack(trackId: string) {
    if (!project.value) return
    const track = tracks.value.find(candidate => candidate.id === trackId)
    if (!track || track.locked) return
    snapshot()
    project.value.tracks = project.value.tracks.filter(candidate => candidate.id !== trackId)
    selectMany(selectedIds.value)
    touched()
  }

  function moveTrack(trackId: string, toIndex: number) {
    if (!project.value) return
    const from = project.value.tracks.findIndex(track => track.id === trackId)
    if (from < 0 || from === toIndex) return
    snapshot()
    const [track] = project.value.tracks.splice(from, 1)
    project.value.tracks.splice(clamp(toIndex, 0, project.value.tracks.length), 0, track)
    touched()
  }

  function removeEmptyTracks() {
    if (!project.value) return
    project.value.tracks = project.value.tracks.filter(track => track.items.length > 0)
  }

  // ------------------------------------------------------------ captions

  /** Replaces the captions generated from `sourceId` (or all auto captions) with new cues. */
  function applyCaptions(cues: CaptionCue[], options: { sourceId: string; offset: number; speed: number; style?: Partial<TextStyle>; y?: number }) {
    if (!project.value) return 0
    snapshot()
    const groupId = `captions-${options.sourceId}`
    for (const track of project.value.tracks) track.items = track.items.filter(item => item.groupId !== groupId)
    let track = project.value.tracks.find(candidate => candidate.kind === 'text' && candidate.name.startsWith('Captions') && !candidate.locked)
    if (!track) {
      track = createTrack('text', project.value.tracks)
      track.name = 'Captions'
      project.value.tracks.unshift(track)
    }
    const style = { ...DEFAULT_TEXT_STYLE, ...CAPTION_PRESET, ...options.style }
    const items: TimelineItem[] = cues.map(cue => ({
      id: uid('caption'),
      trackId: track!.id,
      kind: 'text' as const,
      name: cue.text.slice(0, 40),
      start: options.offset + cue.start / options.speed,
      duration: Math.max(MIN_CLIP_DURATION, cue.duration / options.speed),
      groupId,
      transform: normalizeTransform({ x: 50, y: options.y ?? 72 }),
      text: { ...style, content: cue.text, highlightIndex: cue.highlightIndex ?? -1 },
      animIn: cue.continued ? undefined : { kind: 'pop', duration: 0.18 }
    }))
    const occupied = track.items.filter(item => items.some(cue => item.start < cue.start + cue.duration && cue.start < item.start + item.duration))
    if (occupied.length) {
      track = createTrack('text', project.value.tracks)
      track.name = 'Captions'
      project.value.tracks.unshift(track)
      items.forEach(item => { item.trackId = track!.id })
    }
    track.items.push(...items)
    repairTrack(track)
    removeEmptyTracks()
    touched()
    return items.length
  }

  /** Copies the text style of the primary caption to every caption of the same group. */
  function applyStyleToGroup() {
    const item = selectedItem.value
    if (!item?.text || !item.groupId) return 0
    snapshot()
    let count = 0
    for (const track of tracks.value) {
      for (const candidate of track.items) {
        if (candidate.groupId !== item.groupId || candidate.id === item.id || candidate.kind !== 'text') continue
        // Word-highlight captions keep which word is lit and stay still between words.
        const highlightIndex = candidate.text?.highlightIndex ?? -1
        const continued = !candidate.animIn && highlightIndex > 0
        candidate.text = { ...structuredClone(toRaw(item.text)), content: candidate.text?.content ?? '', highlightIndex }
        candidate.transform = normalizeTransform(item.transform)
        candidate.animIn = item.animIn && !continued ? { ...item.animIn } : undefined
        candidate.animOut = item.animOut ? { ...item.animOut } : undefined
        count++
      }
    }
    touched()
    return count
  }

  // ------------------------------------------------------------ project settings

  function rename(name: string) {
    if (!project.value || !name.trim() || name.trim() === project.value.name) return
    snapshot()
    project.value.name = name.trim()
    touched()
  }

  function setCanvas(width: number, height: number) {
    if (!project.value || (project.value.width === width && project.value.height === height)) return
    snapshot()
    project.value.width = width
    project.value.height = height
    project.value.export.resolution = Math.min(width, height) >= 1080 ? 1080 : Math.min(width, height)
    touched()
  }

  function setBackground(color: string, record = true) {
    if (!project.value) return
    if (record) snapshot()
    project.value.background = color
    touched()
  }

  function updateExport(patch: Partial<ExportSettings>) {
    if (!project.value) return
    Object.assign(project.value.export, patch)
    if (patch.fps && patch.fps !== project.value.fps) {
      project.value.fps = patch.fps
      tracks.value.forEach(track => repairTransitions(track, patch.fps!))
    }
    touched()
  }

  return {
    project, currentTime, playing, selectedIds, primaryId, zoom, snapping, saveState, revision, thumbnailProvider, offline, checkMedia,
    tracks, fps, frame, assetsById, playbackEnd, timelineLength, selectedItems, selectedItem, selectedTrack, canUndo, canRedo, keyframeTolerance,
    find, open, close, save, flushSave, beginChange, undo, redo,
    play, stop, togglePlay, seek, stepFrames,
    select, selectMany, selectAll,
    addItem, addAsset, addAssetsSequence, registerAssets, updateAsset, removeAsset,
    updateSelected, updateItem, updateText, applyTextPreset, setTransform,
    moveItem, moveItems, trimItem, nudgeSelected, trimToPlayhead,
    splitAt, deleteSelected, duplicateSelected, copySelected, cutSelected, paste, groupSelected, ungroupSelected, toggleItemFlag,
    toggleKeyframeAtPlayhead, jumpKeyframe, setKeyframeEasingAtPlayhead, moveKeyframe, clearKeyframes,
    setAnimation, setTransition, setTransitionForTrack, setEffect, setEffectRange,
    addTrack, updateTrack, removeTrack, moveTrack,
    applyCaptions, applyStyleToGroup,
    rename, setCanvas, setBackground, updateExport
  }
})

export type EditorStore = ReturnType<typeof useEditor>
export type { TimelineMoveMode }

// Keep the store's actions current while editing in dev (no stale instance after hot reload).
if (import.meta.hot) import.meta.hot.accept(acceptHMRUpdate(useEditor, import.meta.hot))
