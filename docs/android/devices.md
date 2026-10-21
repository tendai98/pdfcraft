# Android devices tested

Status: one device run (2026-10-21), the owner's phone. The build machine has no emulator
acceleration and no device attached; device runs are the owner's.

For each device, record the result of the [device checklist](README.md#device-checklist) and the
renderer. The renderer is in logcat at startup (`adb logcat -s pdfcraft | grep renderer:`):
`renderer: wgpu; adapter: <name> (<type>, Vulkan|Gl)` or `renderer: OpenGL (glow)`. If wgpu
fails, record that too, and whether `filesDir/renderer` = `gl` fixed it.

| Device | Android (API) | ABI | Build (commit) | Renderer / backend | Checklist | Notes |
|---|---|---|---|---|---|---|
| Owner's phone (model not recorded) | not recorded | arm64-v8a | `2798a11f` (phone layout, `mobile-layout-3` APK) | not recorded (logcat not captured) | partial: 1 (launch); the owner reports the app working | Phone layout used. Status bar and navigation buttons clear of the app, their icons readable. It took three APKs: the first (built before `3a8a1cb1` was committed) drew under the bars, `3a8a1cb1` kept clear of them but left the strips behind them black (dark icons on black), `2798a11f` paints them. Back, keyboard, Save/Save As and the rest of the checklist not yet reported item by item. |

Device classes the specification asks for: a flagship tablet (Vulkan), a 2021 mid-range phone
(Vulkan, then GLES forced), a low-end API 26–28 device (GLES), and the x86_64 emulator on API 30
and 35 (SwiftShader GLES).

## Revision history

- 2026-10-21: First device: the owner's phone, phone layout and system bars.
- 2026-10-16: Created, empty.
