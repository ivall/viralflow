import { reactive } from 'vue'
import type { MediaAsset, TimelineItem, TransitionKind } from './types'

/**
 * Pointer-based drag & drop between panels. WebView2 suppresses HTML5 drag
 * events while native file drop is enabled, so the editor uses its own.
 */

export type DragPayload =
  | { type: 'asset'; asset: MediaAsset }
  | { type: 'item'; defaults: Partial<TimelineItem> & { kind: TimelineItem['kind'] }; duration: number }
  | { type: 'transition'; kind: TransitionKind }
  | { type: 'files'; paths: string[] }

export interface DragState {
  active: boolean
  payload: DragPayload | null
  label: string
  x: number
  y: number
}

export const drag = reactive<DragState>({ active: false, payload: null, label: '', x: 0, y: 0 })

type DropHandler = (payload: DragPayload, x: number, y: number) => boolean
const targets: Array<{ element: () => HTMLElement | null; handler: DropHandler }> = []

export function registerDropTarget(element: () => HTMLElement | null, handler: DropHandler) {
  const entry = { element, handler }
  targets.push(entry)
  return () => {
    const index = targets.indexOf(entry)
    if (index >= 0) targets.splice(index, 1)
  }
}

export function dropAt(payload: DragPayload, x: number, y: number) {
  const hit = document.elementFromPoint(x, y)
  for (const target of [...targets].reverse()) {
    const element = target.element()
    if (element && hit && element.contains(hit) && target.handler(payload, x, y)) return true
  }
  return false
}

/** Starts a drag after a small movement. Returns false through `onClick` when it was a click. */
export function beginDrag(event: PointerEvent, payload: DragPayload, label: string, onClick?: () => void) {
  if (event.button !== 0) return
  const startX = event.clientX
  const startY = event.clientY
  let started = false
  const move = (moveEvent: PointerEvent) => {
    if (!started && Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) < 5) return
    if (!started) {
      started = true
      drag.active = true
      drag.payload = payload
      drag.label = label
      document.body.style.cursor = 'grabbing'
    }
    drag.x = moveEvent.clientX
    drag.y = moveEvent.clientY
  }
  const finish = (upEvent: PointerEvent) => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', finish)
    window.removeEventListener('keydown', cancel, true)
    document.body.style.cursor = ''
    if (started) {
      const current = drag.payload
      reset()
      if (current) dropAt(current, upEvent.clientX, upEvent.clientY)
    } else onClick?.()
  }
  const cancel = (keyEvent: KeyboardEvent) => {
    if (keyEvent.key !== 'Escape') return
    keyEvent.preventDefault()
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', finish)
    window.removeEventListener('keydown', cancel, true)
    document.body.style.cursor = ''
    reset()
  }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', finish)
  window.addEventListener('keydown', cancel, true)
}

export function reset() {
  drag.active = false
  drag.payload = null
  drag.label = ''
}

/** Updates the shared drag state while files are dragged in from the OS. */
export function setExternalDrag(paths: string[] | null, x = 0, y = 0) {
  if (!paths) {
    if (drag.payload?.type === 'files') reset()
    return
  }
  drag.active = true
  drag.payload = { type: 'files', paths }
  drag.label = paths.length === 1 ? paths[0].split(/[\\/]/).pop() ?? 'File' : `${paths.length} files`
  drag.x = x
  drag.y = y
}

export function dragDuration(payload: DragPayload | null) {
  if (!payload) return 3
  if (payload.type === 'asset') return payload.asset.kind === 'image' ? 4 : payload.asset.duration ?? 5
  if (payload.type === 'item') return payload.duration
  return 3
}

export function dragKind(payload: DragPayload | null): TimelineItem['kind'] | null {
  if (!payload) return null
  if (payload.type === 'asset') return payload.asset.kind
  if (payload.type === 'item') return payload.defaults.kind
  return null
}
