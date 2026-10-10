import { defineStore } from 'pinia'
import { ref } from 'vue'
import { errorMessage, ipc, type Settings, type ToolsStatus } from '@/services/ipc'
import { useToasts } from './toast'

const DEFAULTS: Settings = {
  ffmpegPath: '',
  openrouterApiKey: '',
  openrouterModel: 'openai/gpt-6-luna',
  openrouterTranscriptionModel: 'openai/whisper-large-v3',
  transcriptionProvider: 'local',
  whisperModel: 'base',
  captionLanguage: 'auto',
  defaultAspect: '9:16',
  defaultFps: 30,
  autosave: true,
  snap: true,
  safeArea: false,
  exportFolder: '',
  hardwareEncoding: false
}

export const useSettings = defineStore('settings', () => {
  const value = ref<Settings>({ ...DEFAULTS })
  const tools = ref<ToolsStatus | null>(null)
  const loaded = ref(false)

  async function load() {
    try {
      value.value = { ...DEFAULTS, ...await ipc.getSettings() }
    } catch (error) {
      useToasts().error('Settings could not be loaded', errorMessage(error))
    }
    loaded.value = true
    void refreshTools()
  }

  async function refreshTools() {
    try {
      tools.value = await ipc.toolsStatus()
    } catch {
      tools.value = { found: false, whisper: false, hardwareEncoders: [] }
    }
  }

  async function update(patch: Partial<Settings>) {
    const previousPath = value.value.ffmpegPath
    value.value = { ...value.value, ...patch }
    try {
      value.value = await ipc.saveSettings(value.value)
      if (patch.ffmpegPath !== undefined && patch.ffmpegPath !== previousPath) await refreshTools()
    } catch (error) {
      useToasts().error('Settings could not be saved', errorMessage(error))
    }
  }

  return { value, tools, loaded, load, update, refreshTools }
})
