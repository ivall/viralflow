<script setup lang="ts">
import { Check, ChevronDown, Search } from '@lucide/vue'
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from 'reka-ui'
import { computed, nextTick, ref, watch } from 'vue'
import { BUNDLED_FONTS, cssFamily, type FontEntry } from '@/lib/fonts'

const props = defineProps<{ modelValue: string; installed: FontEntry[]; loading?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [family: string] }>()

const open = ref(false)
const query = ref('')
const active = ref(0)
const list = ref<HTMLElement | null>(null)

const known = computed(() => [...BUNDLED_FONTS, ...props.installed].some(font => font.family === props.modelValue))
const groups = computed(() => {
  const needle = query.value.trim().toLowerCase()
  const match = (font: FontEntry) => !needle || font.family.toLowerCase().includes(needle)
  return [
    { label: 'Included', fonts: BUNDLED_FONTS.filter(match) },
    { label: 'On this computer', fonts: props.installed.filter(match) }
  ].filter(group => group.fonts.length)
})
const flat = computed(() => groups.value.flatMap(group => group.fonts))

watch(query, () => { active.value = 0 })
watch(open, async value => {
  if (!value) return
  query.value = ''
  active.value = Math.max(0, flat.value.findIndex(font => font.family === props.modelValue))
  await nextTick()
  scrollToActive()
})

function scrollToActive() {
  list.value?.querySelector(`[data-index="${active.value}"]`)?.scrollIntoView({ block: 'nearest' })
}

function choose(font: FontEntry) {
  emit('update:modelValue', font.family)
  open.value = false
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    const count = flat.value.length
    if (!count) return
    active.value = (active.value + (event.key === 'ArrowDown' ? 1 : -1) + count) % count
    void nextTick(scrollToActive)
  } else if (event.key === 'Enter') {
    event.preventDefault()
    const font = flat.value[active.value]
    if (font) choose(font)
  }
}
</script>

<template>
  <PopoverRoot v-model:open="open">
    <PopoverTrigger
      aria-label="Font family"
      class="flex h-8 w-full min-w-0 items-center justify-between gap-2 rounded-md border border-line bg-app px-2.5 text-left text-xs text-fg outline-none transition-colors hover:border-line-strong focus-visible:border-accent data-[state=open]:border-accent"
    >
      <span class="truncate" :style="{ fontFamily: cssFamily(modelValue) }">{{ modelValue }}</span>
      <span v-if="!known && !loading" class="ml-auto shrink-0 text-2xs text-faint">Not installed</span>
      <ChevronDown class="size-3.5 shrink-0 text-muted" />
    </PopoverTrigger>
    <PopoverPortal>
      <PopoverContent
        side="bottom" align="start" :side-offset="4"
        class="z-[200] flex max-h-[min(420px,var(--reka-popover-content-available-height))] w-[var(--reka-popover-trigger-width)] min-w-[260px] flex-col overflow-hidden rounded-lg border border-line bg-raised shadow-2xl"
        @open-auto-focus.prevent="($event.target as HTMLElement)?.querySelector('input')?.focus()"
      >
        <div class="flex h-9 shrink-0 items-center gap-2 border-b border-line px-2.5">
          <Search class="size-3.5 shrink-0 text-muted" />
          <input v-model="query" class="min-w-0 flex-1 bg-transparent text-xs text-fg outline-none placeholder:text-faint" placeholder="Search fonts" aria-label="Search fonts" @keydown="onKey">
        </div>
        <div ref="list" class="min-h-0 flex-1 overflow-y-auto p-1" role="listbox" aria-label="Fonts">
          <template v-for="group in groups" :key="group.label">
            <p class="px-2 pb-1 pt-2 text-2xs font-medium text-faint">{{ group.label }}</p>
            <button
              v-for="font in group.fonts" :key="font.family"
              role="option" :aria-selected="font.family === modelValue" :data-index="flat.indexOf(font)"
              class="relative flex h-8 w-full items-center rounded-md pl-7 pr-3 text-left text-[13px] text-fg outline-none"
              :class="flat.indexOf(font) === active ? 'bg-hover' : ''"
              @pointermove="active = flat.indexOf(font)"
              @click="choose(font)"
            >
              <Check v-if="font.family === modelValue" class="absolute left-2 size-3.5 text-accent" />
              <span class="truncate" :style="{ fontFamily: cssFamily(font.family) }">{{ font.family }}</span>
            </button>
          </template>
          <p v-if="loading" class="px-2 py-3 text-2xs text-faint">Loading installed fonts…</p>
          <p v-else-if="!groups.length" class="px-2 py-3 text-2xs text-faint">No fonts match “{{ query }}”.</p>
        </div>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
