<p align="center">
  <img src="docs/peek-logo.svg" width="120" alt="Peek" />
</p>

<h1 align="center">Peek</h1>

<p align="center">
  A little window onto your cameras. Peek pops a live feed into the corner of
  your screen the moment <a href="https://www.home-assistant.io">Home Assistant</a>
  detects motion — and can keep your favorite cameras on screen all the time.
</p>

---

## Features

- **Live, low-latency feeds** over WebRTC using Home Assistant's built-in
  [go2rtc](https://github.com/AlexxIT/go2rtc). On your LAN the video flows
  peer-to-peer, so it stays smooth.
- **Local-first** connection with automatic fallback to your Nabu Casa / remote
  URL when you're away from home (one access token works for both).
- **Motion pop-ups** — any camera you enable appears the instant its motion
  sensor trips, and dismisses on a delay you choose. No limit on how many can
  pop up at once.
- **Per-camera "Keep visible"** — pin your favorite cameras so they're always on
  screen (up to 3 at a time), independent of motion.
- **Drag anywhere** — keep-visible feeds can be click-dragged freely around your
  desktop; each remembers where you put it.
- **Tidy overlays** — frameless, rounded, always-on-top cards. Toggle the
  on-feed labels and audio on or off from the menu bar.
- **Menu-bar app** — no dock icon, no window clutter; everything is driven from
  the tray.
- Built with **Tauri** (Rust + the system WebView): small binary, low memory.
- Works with Reolink (or any camera) exposed to Home Assistant. macOS & Windows.

## How it works

```
Home Assistant ──(WebSocket: motion triggers)──► Rust backend ──events──► overlay WebViews
      │                                                                         │
      └──(WebSocket: camera/webrtc/offer ↔ go2rtc WebRTC media)─────────────────┘
```

A small Rust backend holds one WebSocket to Home Assistant (local URL first,
cloud URL as fallback). It subscribes to your cameras' motion `binary_sensor`
entities and decides which feeds should be on screen (keep-visible cameras plus
whatever currently has motion). Each visible camera gets its own overlay window
that runs a standard `RTCPeerConnection`; the backend relays the WebRTC offer /
answer / ICE for that window through Home Assistant's go2rtc backend.

## Requirements

**To run from source**

- [Rust](https://www.rust-lang.org/tools/install) (stable) + the Tauri
  [system prerequisites](https://tauri.app/start/prerequisites/)
- [Node.js](https://nodejs.org) 18+ (for the Tauri CLI and frontend build)

**Home Assistant side**

- Home Assistant **2024.11+** (ships go2rtc and the WebRTC WebSocket API)
- Each camera added to Home Assistant with a `camera.*` entity and one or more
  motion `binary_sensor.*` entities (the Reolink integration provides these)
- A [long-lived access token](https://www.home-assistant.io/docs/authentication/#your-account-profile)
  (Profile → Security → Long-lived access tokens)

## Setup

On first launch a setup window opens automatically. Enter your Home Assistant
**local URL**, an optional **remote / Nabu Casa URL**, and your **access token**,
then click **Load cameras** to pick each camera and its motion sensors from
dropdowns. Settings are saved to the app's config folder and can be changed any
time from the menu bar (**Settings…**).

## Using Peek

Everything lives in the menu-bar (tray) icon:

| Menu item | What it does |
| --- | --- |
| **Cameras** | Enable/disable which cameras pop up on motion |
| **Keep visible** | Pin a camera so it's always on screen (max 3) |
| **Sound** | Mute/unmute feed audio |
| **Show labels** | Show or hide the camera name + motion badge on feeds |
| **Dismiss after** | How long a motion feed lingers after motion clears |
| **Settings…** | Reopen the setup window |
| **Quit** | Exit Peek |

- **Drag** a keep-visible feed anywhere on your desktop; it stays put.
- Click a feed's **✕** to close it — for a keep-visible camera this also unchecks
  its "Keep visible" so it won't reopen.
- If a keep-visible camera also detects motion, its existing feed simply updates
  its label — no duplicate window appears.

## Run

```bash
npm install      # installs the Tauri CLI + frontend deps
npm run dev      # build + launch (first run compiles the Rust deps)
```

## Build

```bash
npm run build    # produces a .app/.dmg (macOS) or installer (Windows)
```

## Project layout

```
src/                 frontend (React + TypeScript + Vite + Tailwind)
  overlay/           the feed card (native RTCPeerConnection)
  setup/             first-run / settings window
  lib/tauri.ts       typed wrapper over the Tauri commands + events
src-tauri/           Rust backend
  src/lib.rs         windows, tray, state, commands, HA session loop
  src/ha.rs          Home Assistant WebSocket client
```

## Configuration

The setup window writes `config.json` (in the app's config folder). Keys:

| Key | Description |
| --- | --- |
| `haUrl` | Home Assistant local URL, e.g. `http://homeassistant.local:8123` |
| `cloudUrl` | Optional remote / Nabu Casa URL used when local is unreachable |
| `token` | Long-lived access token |
| `cameras` | Array of `{ name, cameraEntity, motionEntities: [] }` |
| `corner` | `top-right`, `top-left`, `bottom-right`, `bottom-left` |
| `margin` | Distance from the screen edge, in pixels |
| `width`, `height` | Overlay size in pixels |
| `dismissSeconds` | Seconds a motion feed stays after motion clears (`0` = until closed) |

## License

[Apache 2.0](LICENSE)
