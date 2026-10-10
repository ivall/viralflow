<script setup lang="ts">
import { CircleAlert, CircleCheck, Info, X } from '@lucide/vue'
import { useToasts } from '@/stores/toast'

const toasts = useToasts()
</script>

<template>
  <div class="pointer-events-none fixed bottom-4 right-4 z-[300] flex w-[340px] flex-col gap-2" aria-live="polite">
    <TransitionGroup enter-from-class="translate-y-2 opacity-0" leave-to-class="opacity-0" enter-active-class="transition duration-200" leave-active-class="transition duration-150">
      <div v-for="toast in toasts.items" :key="toast.id" class="pointer-events-auto flex gap-3 rounded-lg border border-line bg-raised px-3 py-2.5 shadow-2xl">
        <CircleCheck v-if="toast.tone === 'success'" class="mt-0.5 size-4 shrink-0 text-ok" />
        <CircleAlert v-else-if="toast.tone === 'error'" class="mt-0.5 size-4 shrink-0 text-danger" />
        <Info v-else class="mt-0.5 size-4 shrink-0 text-accent" />
        <div class="min-w-0 flex-1">
          <p class="text-xs font-medium text-fg">{{ toast.title }}</p>
          <p v-if="toast.detail" class="mt-0.5 break-words text-2xs text-muted">{{ toast.detail }}</p>
          <button v-if="toast.action" class="mt-1.5 text-2xs font-semibold text-accent hover:underline" @click="toast.action.run(); toasts.dismiss(toast.id)">{{ toast.action.label }}</button>
        </div>
        <button class="grid size-5 shrink-0 place-items-center rounded text-faint hover:text-fg" aria-label="Dismiss" @click="toasts.dismiss(toast.id)"><X class="size-3.5" /></button>
      </div>
    </TransitionGroup>
  </div>
</template>
