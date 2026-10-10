export const SHORTCUTS: Array<{ group: string; items: Array<[string, string]> }> = [
  {
    group: 'Playback',
    items: [
      ['Space', 'Play / pause'],
      ['← / →', 'Previous / next frame'],
      ['Shift + ← / →', 'Back / forward 1 second'],
      ['Home / End', 'Go to start / end']
    ]
  },
  {
    group: 'Editing',
    items: [
      ['S  or  Ctrl + B', 'Split at playhead'],
      ['Q / W', 'Trim start / end to playhead'],
      ['Delete', 'Delete selection'],
      ['Shift + Delete', 'Delete and close the gap'],
      ['Ctrl + D', 'Duplicate'],
      ['Ctrl + C / X / V', 'Copy / cut / paste at playhead'],
      ['Alt + ← / →', 'Nudge selection by one frame'],
      ['Ctrl + G / Ctrl + Shift + G', 'Group / ungroup'],
      ['Ctrl + A', 'Select all'],
      ['Esc', 'Clear selection']
    ]
  },
  {
    group: 'Keyframes',
    items: [
      ['K', 'Add / remove keyframe'],
      ['Shift + K / Alt + K', 'Previous / next keyframe']
    ]
  },
  {
    group: 'Project',
    items: [
      ['Ctrl + Z', 'Undo'],
      ['Ctrl + Shift + Z  or  Ctrl + Y', 'Redo'],
      ['Ctrl + I', 'Import media'],
      ['Ctrl + S', 'Save'],
      ['Ctrl + E', 'Export'],
      ['= / −', 'Zoom timeline in / out'],
      ['Shift + Z', 'Fit timeline'],
      ['Ctrl + scroll', 'Zoom timeline at the pointer']
    ]
  }
]
