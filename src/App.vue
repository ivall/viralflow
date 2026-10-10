<script setup lang="ts">
import { TooltipProvider } from 'reka-ui'
import { onMounted } from 'vue'
import UiToaster from '@/components/ui/UiToaster.vue'
import { useSettings } from '@/stores/settings'

const settings = useSettings()
onMounted(() => settings.load())
</script>

<template>
  <TooltipProvider :delay-duration="450">
    <RouterView v-if="settings.loaded" />
    <UiToaster />
  </TooltipProvider>
</template>
