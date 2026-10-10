import { ipc } from '@/services/ipc'

export interface FontEntry {
  family: string
  bundled: boolean
  /** Installed weights; undefined for bundled variable fonts, which cover 400-900. */
  weights?: number[]
}

/** Fonts that ship with the app, so projects look the same on every computer. */
export const BUNDLED_FONTS: FontEntry[] = [
  'Inter', 'Manrope', 'DM Sans', 'Montserrat', 'Poppins', 'Rubik', 'Roboto Condensed', 'Barlow Condensed', 'Oswald', 'Anton', 'Bebas Neue', 'Archivo Black'
].map(family => ({ family, bundled: true }))

export const WEIGHT_NAMES: Record<number, string> = {
  100: 'Thin', 200: 'Extra light', 300: 'Light', 400: 'Regular', 500: 'Medium', 600: 'Semibold', 700: 'Bold', 800: 'Extra bold', 900: 'Black'
}

export function cssFamily(family: string) {
  return `"${family.replace(/["\\]/g, '')}"`
}

let probe: CanvasRenderingContext2D | null = null

/** True when the webview can draw the family by name (it renders both preview and export text). */
function renders(family: string) {
  probe ??= document.createElement('canvas').getContext('2d')
  if (!probe) return true
  const sample = 'mmmmmmmmmmlli1WQ@#Ag'
  return ['monospace', 'serif'].some(fallback => {
    probe!.font = `72px ${fallback}`
    const base = probe!.measureText(sample).width
    probe!.font = `72px ${cssFamily(family)}, ${fallback}`
    return probe!.measureText(sample).width !== base
  })
}

let library: Promise<FontEntry[]> | null = null

/** Installed fonts the webview can use, without the bundled ones. Loaded once per session. */
export function systemFonts(): Promise<FontEntry[]> {
  library ??= ipc.systemFonts()
    .then(families => {
      const bundled = new Set(BUNDLED_FONTS.map(font => font.family.toLowerCase()))
      const seen = new Set<string>()
      const result: FontEntry[] = []
      for (const font of families) {
        const family = [font.name, ...font.aliases].find(renders)
        if (!family || bundled.has(family.toLowerCase()) || seen.has(family.toLowerCase())) continue
        seen.add(family.toLowerCase())
        result.push({ family, bundled: false, weights: font.weights.map(weight => Math.min(900, Math.max(100, Math.round(weight / 100) * 100))) })
      }
      return result.sort((a, b) => a.family.localeCompare(b.family))
    })
    .catch(() => [])
  return library
}
