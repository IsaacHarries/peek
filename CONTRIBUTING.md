# Contributing

Thanks for your interest in improving Peek! Issues, ideas and pull requests are welcome.

## Getting started

```bash
git clone https://github.com/IsaacHarries/peek
cd peek
npm install
npm run dev
```

On first launch a setup window lets you connect to Home Assistant (local URL,
optional Nabu Casa / remote URL, and a long-lived access token) and pick your
cameras from dropdowns — no file editing required.

## Stack

- **Frontend:** React + TypeScript + Vite + Tailwind (in `src/`).
- **Backend:** Rust + Tauri 2 (in `src-tauri/`), talking to Home Assistant over
  its WebSocket API and streaming via go2rtc WebRTC.

## Building

```bash
npm run build   # .app/.dmg on macOS, installer on Windows
```

## Guidelines

- Keep it lightweight, match the existing style, and keep changes focused.
- Never commit `config.json` or any personal data (tokens, entity ids) — it is
  gitignored.
- For larger changes, open an issue or discussion first so we can align.

## Reporting bugs

Open an issue with your OS, Home Assistant version, and steps to reproduce.
Terminal output from `npm run dev` helps a lot.
