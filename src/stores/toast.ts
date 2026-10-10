import { defineStore } from 'pinia'
import { ref } from 'vue'

export type ToastTone = 'info' | 'success' | 'error'

export interface Toast {
  id: number
  tone: ToastTone
  title: string
  detail?: string
  action?: { label: string; run: () => void }
}

let nextId = 1

export const useToasts = defineStore('toasts', () => {
  const items = ref<Toast[]>([])

  function dismiss(id: number) {
    items.value = items.value.filter(toast => toast.id !== id)
  }

  function show(title: string, options: { tone?: ToastTone; detail?: string; action?: Toast['action']; timeout?: number } = {}) {
    const toast: Toast = { id: nextId++, tone: options.tone ?? 'info', title, detail: options.detail, action: options.action }
    items.value = [...items.value.slice(-3), toast]
    const timeout = options.timeout ?? (toast.tone === 'error' ? 7000 : 3500)
    if (timeout > 0) setTimeout(() => dismiss(toast.id), timeout)
    return toast.id
  }

  return {
    items,
    dismiss,
    show,
    success: (title: string, detail?: string) => show(title, { tone: 'success', detail }),
    error: (title: string, detail?: string) => show(title, { tone: 'error', detail })
  }
})
