<script setup lang="ts">
import { FolderOpen, Link, Plus, Trash2, Upload } from '@lucide/vue'
import { computed, ref } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiContextMenu from '@/components/ui/UiContextMenu.vue'
import UiSegmented from '@/components/ui/UiSegmented.vue'
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

const filterOptions = computed(() => [{ value: 'all' as const, label: 'All' }, ...props.kinds.map(kind => ({ value: kind, label: kind === 'video' ? 'Video' : kind === 'image' ? 'Photos' : 'Audio' }))])
const assets = computed(() => (editor.project?.assets ?? []).filter(asset => props.kinds.includes(asset.kind) && (filter.value === 'all' || asset.kind === filter.value)).slice().reverse())
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
  <div class="flex h-full flex-col">
    <div class="space-y-2 p-3">
      <UiButton variant="primary" class="w-full" @click="importFiles"><Upload class="size-3.5" />Import</UiButton>
      <UiSegmented v-if="kinds.length > 1" v-model="filter" :options="filterOptions" label="Media type" />
    </div>
    <div class="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
      <UiContextMenu v-if="assets.length" :items="menu" @select="onMenu">
        <div class="grid grid-cols-2 gap-x-2 gap-y-3">
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
      <div v-else class="mt-6 flex flex-col items-center px-4 text-center">
        <p class="text-xs font-medium text-fg">No media yet</p>
        <p class="mt-1 text-2xs leading-4 text-muted">Import files or drag them from your computer. They are used from their current location - nothing is copied.</p>
      </div>
    </div>
  </div>
</template>
