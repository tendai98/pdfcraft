# Android devices tested

Status: no device tested yet (2026-10-16). The build machine has no emulator acceleration and no
device attached; the first runs are the owner's.

For each device, record the result of the [device checklist](README.md#device-checklist) and the
renderer. The renderer is in logcat at startup (`adb logcat -s pdfcraft | grep renderer:`):
`renderer: wgpu; adapter: <name> (<type>, Vulkan|Gl)` or `renderer: OpenGL (glow)`. If wgpu
fails, record that too, and whether `filesDir/renderer` = `gl` fixed it.

| Device | Android (API) | ABI | Build (commit) | Renderer / backend | Checklist | Notes |
|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — |

Device classes the specification asks for: a flagship tablet (Vulkan), a 2021 mid-range phone
(Vulkan, then GLES forced), a low-end API 26–28 device (GLES), and the x86_64 emulator on API 30
and 35 (SwiftShader GLES).

## Revision history

- 2026-10-16: Created, empty.
