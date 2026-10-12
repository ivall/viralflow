<script setup lang="ts">
import { AudioLines, Play, Plus, Square } from '@lucide/vue'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { beginDrag } from '@/lib/dnd'
import { errorMessage, fileUrl, ipc } from '@/services/ipc'
import { formatDuration, importIntoProject } from '@/services/media'
import { useEditor } from '@/stores/editor'
import { useToasts } from '@/stores/toast'
import MediaPanel from './MediaPanel.vue'

const editor = useEditor()
const sounds = ref<Array<{ name: string; path: string; duration: number }>>([])
const playingPath = ref<string | null>(null)
let audition: HTMLAudioElement | null = null

onMounted(async () => {
  try {
    sounds.value = await ipc.builtinSounds()
  } catch (error) {
    useToasts().error('Sound effects are unavailable', errorMessage(error))
  }
})

onBeforeUnmount(() => audition?.pause())

function listen(path: string) {
  audition?.pause()
  if (playingPath.value === path) {
    playingPath.value = null
    return
  }
  audition = new Audio(fileUrl(path))
  audition.onended = () => { playingPath.value = null }
  void audition.play()
  playingPath.value = path
}

async function assetFor(path: string) {
  const [asset] = await importIntoProject([path])
  return asset
}

async function add(path: string) {
  const asset = await assetFor(path)
  if (asset) editor.addAsset(asset)
}

async function onPointer(event: PointerEvent, sound: { name: string; path: string; duration: number }) {
  const existing = editor.project?.assets.find(asset => asset.path === sound.path)
  if (existing) beginDrag(event, { type: 'asset', asset: existing }, sound.name)
  else beginDrag(event, { type: 'files', paths: [sound.path] }, sound.name)
}
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="h-[52%] min-h-0 border-b border-line"><MediaPanel :kinds="['audio']" /></div>
    <div class="min-h-0 flex-1 overflow-y-auto p-3">
      <h3 class="mb-2 text-xs font-semibold">Sound effects</h3>
      <ul class="space-y-1">
        <li
          v-for="sound in sounds" :key="sound.path"
          class="group flex h-9 items-center gap-2 rounded-md px-2 hover:bg-hover"
          @pointerdown="onPointer($event, sound)"
          @dblclick="add(sound.path)"
        >
          <button class="grid size-6 place-items-center rounded-full bg-raised text-fg hover:bg-accent hover:text-accent-ink" :aria-label="playingPath === sound.path ? 'Stop' : 'Listen'" @pointerdown.stop @click="listen(sound.path)">
            <Square v-if="playingPath === sound.path" class="size-2.5 fill-current" /><Play v-else class="size-3 fill-current" />
          </button>
          <AudioLines class="size-3.5 text-clip-audio" />
          <span class="min-w-0 flex-1 truncate text-xs">{{ sound.name }}</span>
          <span class="text-[11px] tabular-nums text-faint">{{ formatDuration(sound.duration) }}</span>
          <button class="grid size-6 place-items-center rounded text-muted opacity-0 hover:bg-raised hover:text-fg group-hover:opacity-100" aria-label="Add at playhead" @pointerdown.stop @click="add(sound.path)"><Plus class="size-3.5" /></button>
        </li>
      </ul>
    </div>
  </div>
</template>
