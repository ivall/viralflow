import { acceptHMRUpdate, defineStore } from 'pinia'
import { ref } from 'vue'
import { uid } from '@/lib/engine/math'
import { migrateProject } from '@/lib/migrate'
import type { Project, ProjectSummary } from '@/lib/types'
import { ipc } from '@/services/ipc'

export const useProjects = defineStore('projects', () => {
  const items = ref<ProjectSummary[]>([])
  const loading = ref(false)

  async function refresh() {
    loading.value = true
    try {
      items.value = await ipc.listProjects()
    } finally {
      loading.value = false
    }
  }

  async function create(project: Project) {
    await ipc.saveProject(project)
    await refresh()
    return project
  }

  async function remove(id: string) {
    await ipc.deleteProject(id)
    items.value = items.value.filter(project => project.id !== id)
  }

  async function duplicate(summary: ProjectSummary) {
    const copy = await ipc.duplicateProject(summary.id, uid('project'), `${summary.name} copy`)
    await refresh()
    return copy
  }

  async function rename(id: string, name: string) {
    const project = migrateProject(await ipc.loadProject(id))
    project.name = name.trim() || project.name
    project.updatedAt = new Date().toISOString()
    await ipc.saveProject(project)
    await refresh()
  }

  async function importFile(path: string) {
    const project = migrateProject(await ipc.readProjectFile(path))
    project.id = uid('project')
    project.updatedAt = new Date().toISOString()
    await ipc.saveProject(project)
    await refresh()
    return project
  }

  return { items, loading, refresh, create, remove, duplicate, rename, importFile }
})

// Keep the store's actions current while editing in dev (no stale instance after hot reload).
if (import.meta.hot) import.meta.hot.accept(acceptHMRUpdate(useProjects, import.meta.hot))
