<script setup lang="ts">
import { FolderOpen, Link, Plus, Search, Trash2, Upload, X } from '@lucide/vue'
import { computed, ref } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiContextMenu from '@/components/ui/UiContextMenu.vue'
import type { MenuItem } from '@/components/ui/menu'
import { beginDrag } from '@/lib/dnd'
import type { MediaAsset, MediaKind } from '@/lib/types'
import { ipc } from '@/services/ipc'
import { formatBytes, importIntoProject, pickMediaFiles, relinkAsset } from '@/services/media'
import { useEditor } from '@/stores/editor'
import MediaTile from './MediaTile.vue'

const props = withDefaults(defineProps<{ kinds?: MediaKind[] }>(), { kinds: () => ['video', 'image', 'audio'] })
const editor = useEditor()
const filter = ref<'all' | MediaKind>('all')
const target = ref<MediaAsset | null>(null)

const query = ref('')
const labels: Record<MediaKind, string> = { video: 'Video', image: 'Photos', audio: 'Audio' }

const pool = computed(() => (editor.project?.assets ?? []).filter(asset => props.kinds.includes(asset.kind)))
const categories = computed(() => [
  { value: 'all' as const, label: 'All', count: pool.value.length },
  ...props.kinds.map(kind => ({ value: kind, label: labels[kind], count: pool.value.filter(asset => asset.kind === kind).length }))
])
const assets = computed(() => {
  const needle = query.value.trim().toLowerCase()
  return pool.value
    .filter(asset => (filter.value === 'all' || asset.kind === filter.value) && (!needle || asset.name.toLowerCase().includes(needle)))
    .slice()
    .reverse()
})
const used = computed(() => new Set(editor.tracks.flatMap(track => track.items.map(item => item.assetId)).filter(Boolean)))
const menu = computed<MenuItem[]>(() => [
  { id: 'add', label: 'Add at playhead', icon: Plus, disabled: target.value ? editor.offline.has(target.value.id) : true },
  { id: 'relink', label: 'Relink file…', icon: Link },
  { id: 'reveal', label: 'Show in folder', icon: FolderOpen },
  { id: 'remove', label: 'Remove from project', icon: Trash2, danger: true, separatorBefore: true }
])

async function importFiles() {
  await importIntoProject(await pickMediaFiles())
}

function add(asset: MediaAsset) {
  editor.addAsset(asset)
}

function onPointer(event: PointerEvent, asset: MediaAsset) {
  if (editor.offline.has(asset.id)) return
  beginDrag(event, { type: 'asset', asset }, asset.name)
}

function onMenu(id: string) {
  const asset = target.value
  if (!asset) return
  if (id === 'add') add(asset)
  else if (id === 'relink') void relinkAsset(asset)
  else if (id === 'reveal') void ipc.revealPath(asset.path)
  else if (id === 'remove') editor.removeAsset(asset.id)
}
</script>

<template>
  <div class="flex h-full">
    <nav v-if="kinds.length > 1" class="flex w-[84px] shrink-0 flex-col gap-px border-r border-line px-1.5 py-2.5" role="radiogroup" aria-label="Media type">
      <button
        v-for="category in categories" :key="category.value"
        type="button" role="radio" :aria-checked="filter === category.value"
        class="flex h-7 items-center justify-between gap-1 rounded-md px-2 text-left text-xs transition-colors"
        :class="filter === category.value ? 'bg-hover font-medium text-fg' : 'text-muted hover:text-fg'"
        @click="filter = category.value"
      >
        <span class="truncate">{{ category.label }}</span>
        <span v-if="category.count" class="text-[11px] tabular-nums text-faint">{{ category.count }}</span>
      </button>
    </nav>
    <div class="flex min-w-0 flex-1 flex-col">
      <div class="flex shrink-0 gap-1.5 p-2.5">
        <div class="relative min-w-0 flex-1">
          <Search class="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-faint" />
          <input v-model="query" class="field h-8 pl-8 pr-7" placeholder="Search media" aria-label="Search media" spellcheck="false" @keydown.esc="query = ''">
          <button v-if="query" class="absolute right-1.5 top-1/2 grid size-5 -translate-y-1/2 place-items-center rounded text-muted hover:bg-hover hover:text-fg" aria-label="Clear search" @click="query = ''"><X class="size-3" /></button>
        </div>
        <UiButton variant="primary" icon title="Import media" aria-label="Import media" @click="importFiles"><Upload class="size-3.5" /></UiButton>
      </div>
      <div class="min-h-0 flex-1 overflow-y-auto px-2.5 pb-3">
        <UiContextMenu v-if="assets.length" :items="menu" @select="onMenu">
          <div class="grid grid-cols-[repeat(auto-fill,minmax(104px,1fr))] gap-x-2 gap-y-3">
            <MediaTile
              v-for="asset in assets" :key="asset.id"
              :asset="asset" :offline="editor.offline.has(asset.id)" :used="used.has(asset.id)"
              :title="`${asset.name}\n${asset.width ? `${asset.width}×${asset.height} · ` : ''}${formatBytes(asset.size)}`"
              @pointerdown="onPointer($event, asset)"
              @contextmenu="target = asset"
              @dblclick="!editor.offline.has(asset.id) && add(asset)"
              @add="add(asset)"
            />
          </div>
        </UiContextMenu>
        <div v-else-if="pool.length" class="mt-6 flex flex-col items-center px-4 text-center">
          <p class="text-xs font-medium text-fg">{{ query.trim() ? 'Nothing found' : `No ${filter === 'all' ? 'media' : labels[filter].toLowerCase()} yet` }}</p>
          <p v-if="query.trim()" class="mt-1 text-2xs leading-4 text-muted">No {{ filter === 'all' ? 'media' : labels[filter].toLowerCase() }} matches “{{ query.trim() }}”.</p>
        </div>
        <div v-else class="mt-6 flex flex-col items-center px-4 text-center">
          <p class="text-xs font-medium text-fg">No media yet</p>
          <p class="mt-1 text-2xs leading-4 text-muted">Import files or drag them from your computer. They are used from their current location - nothing is copied.</p>
          <UiButton variant="primary" class="mt-3" @click="importFiles"><Upload class="size-3.5" />Import</UiButton>
        </div>
      </div>
    </div>
  </div>
</template>
