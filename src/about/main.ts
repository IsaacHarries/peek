import { getVersion, getTauriVersion } from "@tauri-apps/api/app";

async function fill() {
  const versionEl = document.getElementById("version");
  const tauriEl = document.getElementById("tauri");
  try {
    const [version, tauri] = await Promise.all([getVersion(), getTauriVersion()]);
    if (versionEl) versionEl.textContent = `Version ${version}`;
    if (tauriEl) tauriEl.textContent = `Tauri ${tauri}`;
  } catch (err) {
    console.warn("Failed to read app version:", err);
    if (versionEl) versionEl.textContent = "Version unknown";
  }
}

fill();
