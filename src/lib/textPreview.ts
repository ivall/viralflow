import type { TextStyle } from './types'

/**
 * CSS that mimics a text style at a small size, for preset cards. `fontSize`
 * is the preview size in CSS pixels; outlines, shadows and boxes scale with it.
 */
export function textPreviewStyle(style: Partial<TextStyle>, fontSize: number) {
  const scale = fontSize / (style.size ?? 84)
  const stroke = style.strokeWidth ? Math.max(1, style.strokeWidth * scale * 2) : 0
  const boxed = Boolean(style.backgroundOpacity)
  const padding = style.backgroundPadding ?? 18
  const shadow = style.shadowOpacity
    ? `0 ${(style.shadowDistance ?? 0) * scale}px ${(style.shadowBlur ?? 0) * scale * 1.4}px ${withAlpha(style.shadowColor ?? '#000000', style.shadowOpacity)}`
    : undefined
  return {
    fontFamily: `'${style.font ?? 'Inter'}', 'Segoe UI Emoji', sans-serif`,
    fontWeight: String(style.weight ?? 800),
    fontStyle: style.italic ? 'italic' : 'normal',
    fontSize: `${fontSize}px`,
    lineHeight: '1.2',
    letterSpacing: style.letterSpacing ? `${style.letterSpacing * scale}px` : undefined,
    color: style.color ?? '#ffffff',
    textTransform: style.uppercase ? 'uppercase' as const : 'none' as const,
    WebkitTextStroke: stroke ? `${stroke}px ${style.strokeColor ?? '#000000'}` : undefined,
    paintOrder: 'stroke fill',
    textShadow: shadow,
    background: boxed ? withAlpha(style.backgroundColor ?? '#000000', style.backgroundOpacity ?? 1) : undefined,
    padding: boxed ? `${Math.max(2, padding * scale * 0.6)}px ${Math.max(5, padding * scale)}px` : undefined,
    borderRadius: boxed ? `${(style.backgroundRadius ?? 0) * scale}px` : undefined,
    boxDecorationBreak: 'clone' as const,
    WebkitBoxDecorationBreak: 'clone' as const
  }
}

function withAlpha(hex: string, alpha: number) {
  const value = /^#[\da-f]{6}$/i.test(hex) ? hex : '#000000'
  const int = Number.parseInt(value.slice(1), 16)
  return `rgba(${(int >> 16) & 255}, ${(int >> 8) & 255}, ${int & 255}, ${Math.max(0, Math.min(1, alpha))})`
}

/** Words of a preview string, with the CSS that makes the highlighted one stand out. */
export function previewWords(style: Partial<TextStyle>, text: string) {
  const words = text.split(/(\s+)/).filter(Boolean)
  const mode = style.highlightMode ?? 'none'
  const target = mode === 'none' ? -1 : Math.min(style.highlightIndex ?? -1, words.filter(word => word.trim()).length - 1)
  let index = -1
  return words.map(word => {
    if (!word.trim()) return { text: word, style: undefined }
    index++
    if (index !== target) return { text: word, style: undefined }
    return {
      text: word,
      style: mode === 'box'
        ? { background: style.highlightColor, color: style.highlightTextColor || undefined, borderRadius: '0.18em', padding: '0 0.16em', margin: '0 -0.04em' }
        : { color: style.highlightColor }
    }
  })
}
