<script setup lang="ts">
import { save as saveDialog, open as openDialog } from '@tauri-apps/plugin-dialog'
import { AudioLines, FileUp, FolderOpen, Plus, Search, Settings, Sparkles, TriangleAlert } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import AiDraftDialog from '@/components/home/AiDraftDialog.vue'
import AutoEditDialog from '@/components/home/AutoEditDialog.vue'
import ProjectCard from '@/components/home/ProjectCard.vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiConfirm from '@/components/ui/UiConfirm.vue'
import UiDialog from '@/components/ui/UiDialog.vue'
import { ASPECT_PRESETS, createProject } from '@/lib/project'
import type { Fps, ProjectSummary } from '@/lib/types'
import { errorMessage, ipc } from '@/services/ipc'
import { useProjects } from '@/stores/projects'
import { useSettings } from '@/stores/settings'
import { useToasts } from '@/stores/toast'

const router = useRouter()
const projects = useProjects()
const settings = useSettings()
const toasts = useToasts()
const query = ref('')
const renaming = ref<ProjectSummary | null>(null)
const renameDraft = ref('')
const deleting = ref<ProjectSummary | null>(null)
const showAiDraft = ref(false)
const showAutoEdit = ref(false)
const creating = ref(false)

const visible = computed(() => {
  const text = query.value.trim().toLowerCase()
  return text ? projects.items.filter(project => project.name.toLowerCase().includes(text)) : projects.items
})
const aiReady = computed(() => settings.key.hasKey)

onMounted(async () => {
  try {
    await projects.refresh()
  } catch (error) {
    toasts.error('Projects could not be loaded', errorMessage(error))
  }
})

async function startProject(width: number, height: number) {
  if (creating.value) return
  creating.value = true
  try {
    const project = createProject({ width, height, fps: settings.value.defaultFps as Fps })
    await projects.create(project)
    await router.push({ name: 'editor', params: { id: project.id } })
  } catch (error) {
    toasts.error('Project could not be created', errorMessage(error))
  } finally {
    creating.value = false
  }
}

function open(project: ProjectSummary) {
  router.push({ name: 'editor', params: { id: project.id } })
}

async function importProjectFile() {
  const path = await openDialog({ title: 'Open project file', filters: [{ name: 'ViralFlow project', extensions: ['vfproj', 'json'] }] })
  if (!path || Array.isArray(path)) return
  try {
    const project = await projects.importFile(path)
    toasts.success('Project imported', project.name)
  } catch (error) {
    toasts.error('This file is not a ViralFlow project', errorMessage(error))
  }
}

async function onAction(project: ProjectSummary, action: string) {
  try {
    if (action === 'rename') {
      renaming.value = project
      renameDraft.value = project.name
    } else if (action === 'duplicate') {
      await projects.duplicate(project)
      toasts.success('Project duplicated')
    } else if (action === 'delete') {
      deleting.value = project
    } else if (action === 'export-file') {
      const path = await saveDialog({ title: 'Save project file', defaultPath: `${project.name}.vfproj`, filters: [{ name: 'ViralFlow project', extensions: ['vfproj'] }] })
      if (!path) return
      await ipc.writeProjectFile(path, await ipc.loadProject(project.id))
      toasts.success('Project file saved', 'Media files are referenced by their location on this computer.')
    }
  } catch (error) {
    toasts.error('Action failed', errorMessage(error))
  }
}

async function confirmRename() {
  if (!renaming.value) return
  const target = renaming.value
  renaming.value = null
  try {
    await projects.rename(target.id, renameDraft.value)
  } catch (error) {
    toasts.error('Project could not be renamed', errorMessage(error))
  }
}

async function confirmDelete() {
  if (!deleting.value) return
  const target = deleting.value
  try {
    await projects.remove(target.id)
    toasts.show('Project deleted', { detail: target.name })
  } catch (error) {
    toasts.error('Project could not be deleted', errorMessage(error))
  }
}

async function openCreated(id: string) {
  await projects.refresh()
  await router.push({ name: 'editor', params: { id } })
}
</script>

<template>
  <div class="flex h-full flex-col bg-app">
    <header class="flex h-12 shrink-0 items-center gap-3 border-b border-line px-5">
      <img src="/app-icon.png" alt="" class="size-6 rounded-md">
      <span class="text-sm font-semibold tracking-tight">ViralFlow</span>
      <UiButton class="ml-auto" variant="ghost" size="sm" @click="router.push({ name: 'settings' })"><Settings class="size-4" />Settings</UiButton>
    </header>

    <main class="min-h-0 flex-1 overflow-y-auto">
      <div class="mx-auto max-w-[1200px] px-8 pb-16 pt-8">
        <div v-if="settings.tools && !settings.tools.found" class="mb-6 flex items-start gap-3 rounded-lg border border-warn/30 bg-warn/[.07] px-4 py-3">
          <TriangleAlert class="mt-0.5 size-4 shrink-0 text-warn" />
          <div class="min-w-0 flex-1 text-xs">
            <p class="font-medium text-fg">FFmpeg is required to import, preview and export media.</p>
            <p class="mt-0.5 text-muted">Install FFmpeg (for example with <code class="text-fg">winget install Gyan.FFmpeg</code>) or point to its folder in Settings.</p>
          </div>
          <UiButton size="sm" variant="outline" @click="router.push({ name: 'settings' })">Open settings</UiButton>
        </div>

        <section>
          <h1 class="text-lg font-semibold tracking-tight">New project</h1>
          <div class="mt-4 grid grid-cols-[repeat(auto-fill,minmax(132px,1fr))] gap-3">
            <button
              v-for="preset in ASPECT_PRESETS" :key="preset.id"
              class="group flex h-[132px] flex-col items-center justify-between rounded-lg border border-line bg-panel p-3 text-left transition-colors hover:border-accent/60 hover:bg-raised"
              @click="startProject(preset.width, preset.height)"
            >
              <span class="flex flex-1 items-center">
                <span class="block rounded-[3px] border-2 border-muted/70 transition-colors group-hover:border-accent" :style="{ width: `${preset.width >= preset.height ? 64 : 64 * preset.width / preset.height}px`, height: `${preset.height >= preset.width ? 56 : 56 * preset.height / preset.width}px` }" />
              </span>
              <span class="w-full">
                <span class="block text-xs font-semibold text-fg">{{ preset.label }}</span>
                <span class="block truncate text-2xs text-muted">{{ preset.hint }}</span>
              </span>
            </button>
          </div>
          <div class="mt-3 flex flex-wrap gap-2">
            <UiButton variant="outline" @click="importProjectFile"><FileUp class="size-3.5" />Open project file</UiButton>
            <UiButton variant="outline" :title="aiReady ? undefined : 'Add an OpenRouter key in Settings'" @click="showAiDraft = true"><Sparkles class="size-3.5" />Draft from a topic</UiButton>
            <UiButton variant="outline" @click="showAutoEdit = true"><AudioLines class="size-3.5" />Auto-edit voiceovers</UiButton>
          </div>
        </section>

        <section class="mt-10">
          <div class="flex items-center gap-3">
            <h2 class="text-sm font-semibold">Projects</h2>
            <span class="text-2xs text-muted">{{ projects.items.length }}</span>
            <label class="relative ml-auto w-64">
              <Search class="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-faint" />
              <input v-model="query" class="field pl-8" placeholder="Search projects" aria-label="Search projects">
            </label>
          </div>
          <div v-if="visible.length" class="mt-4 grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-x-4 gap-y-6">
            <ProjectCard v-for="project in visible" :key="project.id" :project="project" @open="open(project)" @action="onAction(project, $event)" />
          </div>
          <div v-else-if="!projects.loading" class="mt-4 flex flex-col items-center rounded-lg border border-dashed border-line py-14 text-center">
            <FolderOpen class="size-6 text-faint" />
            <p class="mt-3 text-xs font-medium text-fg">{{ query ? 'No project matches your search' : 'No projects yet' }}</p>
            <p class="mt-1 text-2xs text-muted">{{ query ? 'Try a different name.' : 'Pick a format above to start editing.' }}</p>
            <UiButton v-if="!query" class="mt-4" variant="primary" size="sm" @click="startProject(1080, 1920)"><Plus class="size-3.5" />New 9:16 project</UiButton>
          </div>
        </section>
      </div>
    </main>

    <UiDialog :open="Boolean(renaming)" title="Rename project" width="420px" @update:open="value => { if (!value) renaming = null }">
      <form class="px-5 py-4" @submit.prevent="confirmRename">
        <input v-model="renameDraft" class="field" maxlength="120" aria-label="Project name" autofocus>
      </form>
      <template #footer>
        <UiButton variant="ghost" @click="renaming = null">Cancel</UiButton>
        <UiButton variant="primary" :disabled="!renameDraft.trim()" @click="confirmRename">Rename</UiButton>
      </template>
    </UiDialog>

    <UiConfirm :open="Boolean(deleting)" title="Delete project?" :description="`“${deleting?.name}” will be deleted. Your media files stay where they are.`" confirm-label="Delete project" danger @update:open="value => { if (!value) deleting = null }" @confirm="confirmDelete" />
    <AiDraftDialog v-model:open="showAiDraft" @created="openCreated" />
    <AutoEditDialog v-model:open="showAutoEdit" @created="openCreated" />
  </div>
</template>
