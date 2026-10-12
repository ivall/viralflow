<script setup lang="ts">
import { beginDrag } from '@/lib/dnd'
import { SHAPE_PRESETS, shapeDefaults, stickerDefaults } from '@/lib/project'
import type { ShapeKind } from '@/lib/types'
import { useEditor } from '@/stores/editor'

const editor = useEditor()

const GROUPS: Array<{ title: string; emoji: string[] }> = [
  { title: 'Reactions', emoji: ['😂', '🤣', '😍', '🥹', '😎', '🤯', '😱', '🥳', '😭', '😡', '🤔', '🙄', '😴', '🤩', '😇', '🫠'] },
  { title: 'Gestures', emoji: ['👍', '👎', '👏', '🙌', '🙏', '👉', '👈', '👆', '👇', '✌️', '🤞', '💪', '👀', '🫶', '🤝', '✋'] },
  { title: 'Symbols', emoji: ['🔥', '💯', '✨', '⭐', '💥', '💫', '❤️', '💔', '✅', '❌', '⚠️', '❗', '❓', '💡', '🎯', '🚀'] },
  { title: 'Things', emoji: ['🎉', '🎁', '💰', '💎', '🏆', '📈', '📉', '⏰', '📱', '💻', '🎮', '🎧', '📸', '🎬', '🍕', '☕'] },
  { title: 'Arrows', emoji: ['⬆️', '⬇️', '⬅️', '➡️', '↗️', '↘️', '🔄', '🔁', '⤴️', '⤵️', '🔝', '🆕', '🆗', '🔴', '🟢', '🔵'] }
]

function addSticker(emoji: string) {
  editor.addItem(stickerDefaults(emoji))
}

function onSticker(event: PointerEvent, emoji: string) {
  const defaults = stickerDefaults(emoji)
  beginDrag(event, { type: 'item', defaults, duration: 3 }, emoji, () => addSticker(emoji))
}

function onShape(event: PointerEvent, kind: ShapeKind, label: string) {
  const defaults = shapeDefaults(kind)
  beginDrag(event, { type: 'item', defaults, duration: 3 }, label, () => editor.addItem(defaults))
}
</script>

<template>
  <div class="h-full overflow-y-auto p-3">
    <h3 class="mb-2 text-xs font-semibold">Shapes</h3>
    <div class="grid grid-cols-4 gap-1.5">
      <button v-for="shape in SHAPE_PRESETS" :key="shape.kind" class="flex flex-col items-center gap-1 rounded-md p-1.5 hover:bg-hover" :title="shape.label" @pointerdown="onShape($event, shape.kind, shape.label)">
        <span class="grid h-10 w-full place-items-center rounded bg-app">
          <span v-if="shape.kind === 'blur'" class="h-5 w-8 rounded-md border border-white/20 bg-white/10 backdrop-blur" />
          <span v-else class="block" :class="shape.kind === 'circle' ? 'size-6 rounded-full' : shape.kind === 'line' ? 'h-0.5 w-8 rounded-full' : 'h-4 w-8 rounded'" :style="{ background: shape.color }" />
        </span>
        <span class="w-full truncate text-center text-[11px] text-muted">{{ shape.label }}</span>
      </button>
    </div>
    <template v-for="group in GROUPS" :key="group.title">
      <h3 class="mb-2 mt-5 text-xs font-semibold">{{ group.title }}</h3>
      <div class="grid grid-cols-6 gap-1">
        <button
          v-for="emoji in group.emoji" :key="emoji"
          class="grid aspect-square place-items-center rounded-md text-xl transition-transform hover:scale-110 hover:bg-hover"
          style="font-family: 'Segoe UI Emoji', 'Apple Color Emoji', sans-serif"
          :aria-label="`Sticker ${emoji}`"
          @pointerdown="onSticker($event, emoji)"
        >{{ emoji }}</button>
      </div>
    </template>
  </div>
</template>
