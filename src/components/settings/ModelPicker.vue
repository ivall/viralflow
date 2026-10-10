<script setup lang="ts">
import { Braces, Check, Eye, LoaderCircle, RefreshCw, Search } from '@lucide/vue'
import { computed, nextTick, ref, watch } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiDialog from '@/components/ui/UiDialog.vue'
import UiSegmented from '@/components/ui/UiSegmented.vue'
import { errorMessage, ipc, type OpenRouterModel } from '@/services/ipc'

const props = withDefaults(defineProps<{ kind?: 'chat' | 'transcription' }>(), { kind: 'chat' })
const model = defineModel<string>({ required: true })
const speech = computed(() => props.kind === 'transcription')
const open = ref(false)
const query = ref('')
const loading = ref(false)
const error = ref('')
const visionOnly = ref(false)
const jsonOnly = ref(false)
const freeOnly = ref(false)
const sort = ref<'newest' | 'cheapest' | 'name'>('newest')
const input = ref<HTMLInputElement | null>(null)

// Fetched when the picker first opens; reopening reuses it.
const catalogue = ref<OpenRouterModel[]>([])
const current = computed(() => catalogue.value.find(entry => entry.id === model.value))
const sortOptions = computed(() => speech.value
  ? [{ value: 'newest' as const, label: 'Newest' }, { value: 'name' as const, label: 'Name' }]
  : [{ value: 'newest' as const, label: 'Newest' }, { value: 'cheapest' as const, label: 'Cheapest' }, { value: 'name' as const, label: 'Name' }])

const results = computed(() => {
  const words = query.value.toLowerCase().split(/\s+/).filter(Boolean)
  const list = catalogue.value.filter(entry => {
    if (visionOnly.value && !entry.vision) return false
    if (jsonOnly.value && !entry.jsonMode) return false
    if (freeOnly.value && (entry.promptPrice > 0 || entry.completionPrice > 0)) return false
    const haystack = `${entry.id} ${entry.name}`.toLowerCase()
    return words.every(word => haystack.includes(word))
  })
  if (sort.value === 'cheapest' && !speech.value) return [...list].sort((a, b) => a.promptPrice + a.completionPrice - (b.promptPrice + b.completionPrice))
  if (sort.value === 'name') return [...list].sort((a, b) => a.name.localeCompare(b.name))
  return list
})

async function load(force = false) {
  if (catalogue.value.length && !force) return
  loading.value = true
  error.value = ''
  try {
    catalogue.value = await ipc.openrouterModels(speech.value)
  } catch (reason) {
    error.value = errorMessage(reason)
  } finally {
    loading.value = false
  }
}

watch(open, value => {
  if (!value) return
  void load()
  void nextTick(() => input.value?.focus())
})

function price(value: number) {
  if (value === 0) return 'free'
  return value < 0.1 ? `$${value.toFixed(3)}` : `$${value.toFixed(2)}`
}

function context(tokens: number) {
  return tokens >= 1_000_000 ? `${(tokens / 1_000_000).toFixed(tokens % 1_000_000 ? 1 : 0)}M` : `${Math.round(tokens / 1000)}K`
}

function choose(entry: OpenRouterModel) {
  model.value = entry.id
  open.value = false
}

function useTyped() {
  const typed = query.value.trim()
  if (typed.includes('/')) {
    model.value = typed
    open.value = false
  }
}
</script>

<template>
  <button class="field flex items-center gap-2 text-left" type="button" @click="open = true">
    <span class="min-w-0 flex-1 truncate">{{ model || 'Choose a model' }}</span>
    <Eye v-if="current?.vision" class="size-3.5 shrink-0 text-muted" title="Understands images" />
    <Search class="size-3.5 shrink-0 text-muted" />
  </button>

  <UiDialog
    v-model:open="open"
    :title="speech ? 'Choose a speech-to-text model' : 'Choose an AI model'"
    :description="speech ? 'Models that turn speech into text. Providers bill by audio length or tokens - see openrouter.ai for prices. Word timing depends on the model; Whisper models return it reliably.' : 'Prices are per million tokens (input / output). Photo matching needs a model that understands images.'"
    width="760px"
  >
    <div class="flex flex-col gap-3 px-5 pt-4">
      <label class="relative block">
        <Search class="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-faint" />
        <input ref="input" v-model="query" class="field pl-8" :placeholder="speech ? 'Search, e.g. whisper' : 'Search by name or id, e.g. gpt-6 luna'" spellcheck="false" @keydown.enter="results[0] ? choose(results[0]) : useTyped()">
      </label>
      <div class="flex flex-wrap items-center gap-1.5">
        <template v-if="!speech">
        <button class="flex h-7 items-center gap-1.5 rounded-md border px-2.5 text-2xs transition-colors" :class="visionOnly ? 'border-accent bg-accent/10 text-fg' : 'border-line text-muted hover:text-fg'" @click="visionOnly = !visionOnly"><Eye class="size-3" />Images</button>
        <button class="flex h-7 items-center gap-1.5 rounded-md border px-2.5 text-2xs transition-colors" :class="jsonOnly ? 'border-accent bg-accent/10 text-fg' : 'border-line text-muted hover:text-fg'" @click="jsonOnly = !jsonOnly"><Braces class="size-3" />Structured output</button>
        <button class="flex h-7 items-center rounded-md border px-2.5 text-2xs transition-colors" :class="freeOnly ? 'border-accent bg-accent/10 text-fg' : 'border-line text-muted hover:text-fg'" @click="freeOnly = !freeOnly">Free</button>
        </template>
        <span class="ml-auto text-2xs tabular-nums text-faint">{{ results.length }} {{ results.length === 1 ? 'model' : 'models' }}</span>
        <UiSegmented v-model="sort" :class="speech ? 'w-40' : 'w-56'" :options="sortOptions" label="Sort models" />
        <UiButton variant="ghost" size="sm" icon aria-label="Refresh list" :disabled="loading" @click="load(true)"><RefreshCw class="size-3.5" :class="loading && 'animate-spin'" /></UiButton>
      </div>
    </div>
    <div class="mt-3 h-[420px] overflow-y-auto border-t border-line">
      <div v-if="loading && !catalogue.length" class="flex h-full items-center justify-center gap-2 text-xs text-muted"><LoaderCircle class="size-4 animate-spin" />Loading models…</div>
      <div v-else-if="error" class="flex h-full flex-col items-center justify-center gap-2 px-8 text-center">
        <p class="text-xs text-danger">The model list could not be loaded.</p>
        <p class="text-2xs text-muted">{{ error }}</p>
        <UiButton variant="outline" size="sm" @click="load(true)">Try again</UiButton>
      </div>
      <div v-if="!error && catalogue.length && !speech" class="sticky top-0 z-10 grid grid-cols-[1fr_auto_auto_auto] gap-4 border-b border-line bg-panel px-5 py-1.5 text-[10px] text-faint">
        <span>Model</span><span class="w-14 text-right">Context</span><span class="w-16 text-right">Input / 1M</span><span class="w-16 text-right">Output / 1M</span>
      </div>
      <ul v-if="!error && catalogue.length" role="listbox" aria-label="Models">
        <li v-for="entry in results" :key="entry.id" role="option" :aria-selected="entry.id === model">
          <button class="grid w-full items-center gap-4 px-5 py-2 text-left transition-colors hover:bg-hover" :class="[speech ? 'grid-cols-1' : 'grid-cols-[1fr_auto_auto_auto]', entry.id === model && 'bg-accent/10']" @click="choose(entry)">
            <span class="min-w-0">
              <span class="flex items-center gap-1.5 text-xs text-fg">
                <Check v-if="entry.id === model" class="size-3.5 shrink-0 text-accent" />
                <span class="truncate">{{ entry.name }}</span>
                <Eye v-if="entry.vision" class="size-3 shrink-0 text-muted" title="Understands images" />
                <Braces v-if="entry.jsonMode" class="size-3 shrink-0 text-muted" title="Structured output" />
              </span>
              <span class="block truncate text-2xs text-faint">{{ entry.id }}</span>
            </span>
            <template v-if="!speech">
            <span class="w-14 text-right text-2xs tabular-nums text-muted" title="Context window">{{ context(entry.contextLength) }}</span>
            <span class="w-16 text-right text-2xs tabular-nums text-muted" title="Input per 1M tokens">{{ price(entry.promptPrice) }}</span>
            <span class="w-16 text-right text-2xs tabular-nums text-muted" title="Output per 1M tokens">{{ price(entry.completionPrice) }}</span>
            </template>
          </button>
        </li>
        <li v-if="!results.length" class="px-5 py-10 text-center text-xs text-muted">
          No model matches.
          <button v-if="query.includes('/')" class="ml-1 text-accent hover:underline" @click="useTyped">Use “{{ query.trim() }}” anyway</button>
        </li>
      </ul>
    </div>
  </UiDialog>
</template>
