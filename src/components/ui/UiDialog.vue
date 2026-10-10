<script setup lang="ts">
import { X } from '@lucide/vue'
import { DialogClose, DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from 'reka-ui'

const props = withDefaults(defineProps<{ open: boolean; title: string; description?: string; width?: string; dismissible?: boolean }>(), { width: '560px', dismissible: true })
const emit = defineEmits<{ 'update:open': [value: boolean] }>()

function guard(event: Event) {
  if (!props.dismissible) event.preventDefault()
}
</script>

<template>
  <DialogRoot :open="open" @update:open="value => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-[100] bg-black/60" />
      <DialogContent
        class="fixed left-1/2 top-1/2 z-[101] flex max-h-[90vh] w-[calc(100vw-48px)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border border-line bg-panel shadow-2xl outline-none"
        :style="{ maxWidth: width }"
        @escape-key-down="guard"
        @pointer-down-outside="guard"
        @interact-outside="guard"
      >
        <header class="flex items-start gap-3 border-b border-line px-5 py-4">
          <div class="min-w-0 flex-1">
            <DialogTitle class="text-sm font-semibold text-fg">{{ title }}</DialogTitle>
            <DialogDescription v-if="description" class="mt-1 text-xs text-muted">{{ description }}</DialogDescription>
          </div>
          <DialogClose v-if="dismissible" class="grid size-7 place-items-center rounded-md text-muted hover:bg-hover hover:text-fg" aria-label="Close"><X class="size-4" /></DialogClose>
        </header>
        <div class="min-h-0 flex-1 overflow-y-auto"><slot /></div>
        <footer v-if="$slots.footer" class="flex items-center justify-end gap-2 border-t border-line px-5 py-3"><slot name="footer" /></footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
