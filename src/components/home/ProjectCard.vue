<script setup lang="ts">
import { Copy, FileDown, FolderOpen, MoreHorizontal, Pencil, Trash2 } from '@lucide/vue'
import { computed } from 'vue'
import UiContextMenu from '@/components/ui/UiContextMenu.vue'
import UiMenu from '@/components/ui/UiMenu.vue'
import type { MenuItem } from '@/components/ui/menu'
import { aspectLabel } from '@/lib/project'
import type { ProjectSummary } from '@/lib/types'
import { fileUrl } from '@/services/ipc'
import { formatDuration } from '@/services/media'

const props = defineProps<{ project: ProjectSummary }>()
const emit = defineEmits<{ open: []; action: [id: string] }>()

const actions: MenuItem[] = [
  { id: 'open', label: 'Open', icon: FolderOpen },
  { id: 'rename', label: 'Rename', icon: Pencil },
  { id: 'duplicate', label: 'Duplicate', icon: Copy },
  { id: 'export-file', label: 'Save project file…', icon: FileDown },
  { id: 'delete', label: 'Delete', icon: Trash2, danger: true, separatorBefore: true }
]

const portrait = computed(() => props.project.height > props.project.width)
const thumbnail = computed(() => props.project.thumbnail ? `${fileUrl(props.project.thumbnail)}?v=${encodeURIComponent(props.project.updatedAt)}` : '')
const edited = computed(() => {
  const date = new Date(props.project.updatedAt)
  const days = Math.floor((Date.now() - date.getTime()) / 86_400_000)
  if (days <= 0) return `Today, ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
  if (days === 1) return 'Yesterday'
  if (days < 7) return `${days} days ago`
  return date.toLocaleDateString()
})

function onAction(id: string) {
  if (id === 'open') emit('open')
  else emit('action', id)
}
</script>

<template>
  <UiContextMenu :items="actions" @select="onAction">
    <article class="group min-w-0">
      <button class="relative flex aspect-[4/3] w-full items-center justify-center overflow-hidden rounded-lg border border-line bg-panel transition-colors hover:border-line-strong focus-visible:border-accent" :aria-label="`Open ${project.name}`" @click="emit('open')">
        <img v-if="thumbnail" :src="thumbnail" alt="" class="max-h-full max-w-full object-contain" :class="portrait ? 'h-full' : 'w-full'" draggable="false">
        <div v-else class="rounded-md border border-line bg-app" :class="portrait ? 'h-3/4' : 'w-3/4'" :style="{ aspectRatio: `${project.width} / ${project.height}` }" />
        <span class="absolute bottom-2 right-2 rounded bg-black/70 px-1.5 py-0.5 text-2xs tabular-nums text-white">{{ formatDuration(project.duration) }}</span>
        <span class="absolute left-2 top-2 rounded bg-black/60 px-1.5 py-0.5 text-2xs text-white/90">{{ aspectLabel(project.width, project.height) }}</span>
      </button>
      <div class="mt-2 flex items-start gap-1">
        <button class="min-w-0 flex-1 text-left" @click="emit('open')">
          <h3 class="truncate text-xs font-medium text-fg">{{ project.name }}</h3>
          <p class="mt-0.5 text-2xs text-muted">{{ edited }}</p>
        </button>
        <UiMenu :items="actions" @select="onAction">
          <button class="grid size-6 place-items-center rounded text-muted opacity-0 transition-opacity hover:bg-hover hover:text-fg focus-visible:opacity-100 group-hover:opacity-100" aria-label="Project actions"><MoreHorizontal class="size-4" /></button>
        </UiMenu>
      </div>
    </article>
  </UiContextMenu>
</template>
