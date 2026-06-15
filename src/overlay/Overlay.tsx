import { useEffect, useRef, useState, type MouseEvent } from "react";
import { X } from "lucide-react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, type OverlayShow, type UnlistenFn } from "@/lib/tauri";
import { cn } from "@/lib/utils";

const appWindow = getCurrentWebviewWindow();
const osWindow = getCurrentWindow();
const LABEL = appWindow.label;

const DEVICE_CLASS_ICONS: Record<string, string> = {
  motion: "🏃",
  occupancy: "👁",
  presence: "👁",
  moving: "🏃",
  sound: "🔊",
  vibration: "📳",
};

function badgeText(ev: OverlayShow): string {
  const icon = ev.deviceClass ? DEVICE_CLASS_ICONS[ev.deviceClass] : undefined;
  if (icon) return `${icon} ${ev.label || ""}`.trim();
  return ev.label || "";
}

interface Info {
  name: string;
  badge: string;
  detail: string;
  showLabels: boolean;
  draggable: boolean;
}

function infoFrom(ev: OverlayShow): Info {
  return {
    name: ev.name,
    badge: badgeText(ev),
    detail: ev.detail || "",
    showLabels: ev.showLabels !== false,
    draggable: !!ev.draggable,
  };
}

export function Overlay() {
  const [visible, setVisible] = useState(false);
  const [info, setInfo] = useState<Info | null>(null);

  const videoRef = useRef<HTMLVideoElement>(null);
  const pcRef = useRef<RTCPeerConnection | null>(null);
  const remoteReady = useRef(false);
  const bufferedCandidates = useRef<RTCIceCandidateInit[]>([]);
  const hideTimer = useRef<number | null>(null);
  // The entity currently streaming, so we can transparently restart it after
  // the peer connection drops (e.g. the machine slept and the ICE link died).
  const currentEntity = useRef<string | null>(null);
  const restartTimer = useRef<number | null>(null);
  const watchdogTimer = useRef<number | null>(null);
  const lastBytes = useRef<number>(0);
  const stallChecks = useRef<number>(0);

  function clearRestartTimer() {
    if (restartTimer.current) {
      clearTimeout(restartTimer.current);
      restartTimer.current = null;
    }
  }

  function clearWatchdog() {
    if (watchdogTimer.current) {
      clearInterval(watchdogTimer.current);
      watchdogTimer.current = null;
    }
    lastBytes.current = 0;
    stallChecks.current = 0;
  }

  function stopStream() {
    clearRestartTimer();
    clearWatchdog();
    currentEntity.current = null;
    if (pcRef.current) {
      try {
        pcRef.current.close();
      } catch {
        /* ignore */
      }
      pcRef.current = null;
    }
    if (videoRef.current) videoRef.current.srcObject = null;
    remoteReady.current = false;
    bufferedCandidates.current = [];
    api.webrtcStop(LABEL);
  }

  // Re-establish the stream after the connection dropped, but only if this pc
  // is still the active one and the feed is still meant to be on screen.
  function restartStream(pc: RTCPeerConnection) {
    if (pcRef.current !== pc) return;
    const entity = currentEntity.current;
    if (!entity) return;
    startStream(entity);
  }

  // Watch for the peer connection dying (consent freshness lost on sleep, ICE
  // failure, etc.) and restart so a kept-visible feed doesn't freeze forever.
  function watchConnection(pc: RTCPeerConnection) {
    const onChange = () => {
      if (pcRef.current !== pc) return;
      const state = pc.connectionState;
      if (state === "failed") {
        clearRestartTimer();
        restartStream(pc);
      } else if (state === "disconnected") {
        // "disconnected" is often transient and can recover on its own; give
        // it a moment before forcing a full restart.
        if (restartTimer.current) return;
        restartTimer.current = window.setTimeout(() => {
          restartTimer.current = null;
          if (pcRef.current === pc && pc.connectionState !== "connected") restartStream(pc);
        }, 4000);
      } else if (state === "connected") {
        clearRestartTimer();
      }
    };
    pc.onconnectionstatechange = onChange;
    // Older WebRTC stacks surface recovery only via iceConnectionState.
    pc.oniceconnectionstatechange = () => {
      if (pcRef.current !== pc) return;
      const ice = pc.iceConnectionState;
      if (ice === "failed") {
        clearRestartTimer();
        restartStream(pc);
      }
    };
  }

  // After sleep/wake the connection can stay "connected" while RTP has silently
  // stopped, leaving a frozen frame. Poll inbound stats and restart if no new
  // video bytes arrive for several seconds while the feed should be playing.
  function startWatchdog(pc: RTCPeerConnection) {
    clearWatchdog();
    lastBytes.current = 0;
    stallChecks.current = 0;
    watchdogTimer.current = window.setInterval(async () => {
      if (pcRef.current !== pc) return;
      // Only judge stalls once media is meant to be flowing.
      if (!remoteReady.current) return;
      let bytes = 0;
      try {
        const stats = await pc.getStats();
        stats.forEach((report) => {
          if (report.type === "inbound-rtp" && report.kind === "video") {
            bytes = Math.max(bytes, (report as RTCInboundRtpStreamStats).bytesReceived ?? 0);
          }
        });
      } catch {
        return;
      }
      if (pcRef.current !== pc) return;
      if (bytes > lastBytes.current) {
        lastBytes.current = bytes;
        stallChecks.current = 0;
        return;
      }
      // No growth since last poll. Wait for a couple of stalled polls (to ride
      // out brief hiccups) before forcing a restart.
      stallChecks.current += 1;
      if (stallChecks.current >= 3) {
        restartStream(pc);
      }
    }, 3000);
  }

  async function startStream(entity: string) {
    stopStream();
    currentEntity.current = entity;
    remoteReady.current = false;
    bufferedCandidates.current = [];
    const pc = new RTCPeerConnection();
    pcRef.current = pc;
    pc.addTransceiver("video", { direction: "recvonly" });
    pc.addTransceiver("audio", { direction: "recvonly" });
    pc.ontrack = (e) => {
      const v = videoRef.current;
      if (v && e.streams[0] && v.srcObject !== e.streams[0]) v.srcObject = e.streams[0];
    };
    pc.onicecandidate = (e) => {
      if (e.candidate) api.webrtcCandidate(LABEL, e.candidate.toJSON());
    };
    watchConnection(pc);
    startWatchdog(pc);
    try {
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      if (offer.sdp) api.webrtcOffer(LABEL, entity, offer.sdp);
    } catch (err) {
      console.warn("Failed to create WebRTC offer:", err);
    }
  }

  function doShow(ev: OverlayShow) {
    if (hideTimer.current) {
      clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
    setInfo(infoFrom(ev));
    if (videoRef.current) videoRef.current.muted = !ev.sound;
    startStream(ev.cameraEntity);
    setVisible(true);
    api.overlayPresent(LABEL);
  }

  // Update labels/badge/drag state without restarting the stream.
  function doUpdate(ev: OverlayShow) {
    setInfo(infoFrom(ev));
    if (videoRef.current) videoRef.current.muted = !ev.sound;
  }

  function doHide() {
    if (hideTimer.current) clearTimeout(hideTimer.current);
    setVisible(false);
    hideTimer.current = window.setTimeout(() => {
      stopStream();
      api.overlayHide(LABEL);
    }, 320);
  }

  function onClose() {
    // Closing turns off this camera's keep-visible so it does not re-open.
    api.overlayClose(LABEL);
    doHide();
  }

  useEffect(() => {
    let unlisteners: UnlistenFn[] = [];
    let cancelled = false;

    (async () => {
      unlisteners = await Promise.all([
        appWindow.listen<OverlayShow>("overlay-show", (e) => doShow(e.payload)),
        appWindow.listen<OverlayShow>("overlay-update", (e) => doUpdate(e.payload)),
        appWindow.listen("overlay-teardown", () => doHide()),
        appWindow.listen<{ sound: boolean }>("overlay-sound", (e) => {
          if (videoRef.current) videoRef.current.muted = !e.payload.sound;
        }),
        appWindow.listen<{ show: boolean }>("overlay-labels", (e) => {
          setInfo((prev) => (prev ? { ...prev, showLabels: e.payload.show } : prev));
        }),
        appWindow.listen<{ sdp: string }>("webrtc-answer", async (e) => {
          const pc = pcRef.current;
          if (!pc) return;
          try {
            await pc.setRemoteDescription({ type: "answer", sdp: e.payload.sdp });
            remoteReady.current = true;
            for (const c of bufferedCandidates.current) pc.addIceCandidate(c).catch(() => {});
            bufferedCandidates.current = [];
          } catch (err) {
            console.warn("setRemoteDescription failed:", err);
          }
        }),
        appWindow.listen<{ candidate: RTCIceCandidateInit }>("webrtc-remote-candidate", (e) => {
          const pc = pcRef.current;
          const candidate = e.payload.candidate;
          if (!pc || !candidate) return;
          if (remoteReady.current) pc.addIceCandidate(candidate).catch(() => {});
          else bufferedCandidates.current.push(candidate);
        }),
        appWindow.listen<{ message: string }>("webrtc-error", (e) => {
          console.warn("Home Assistant WebRTC error:", e.payload?.message);
        }),
      ]);

      if (cancelled) {
        unlisteners.forEach((un) => un());
        return;
      }
      api.overlayReady(LABEL);
    })();

    return () => {
      cancelled = true;
      unlisteners.forEach((un) => un());
      stopStream();
      if (hideTimer.current) clearTimeout(hideTimer.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const noLabels = !info?.showLabels;

  function onCardMouseDown(e: MouseEvent) {
    if (!info?.draggable || e.button !== 0) return;
    // Don't start a drag when the click is on the close button.
    if ((e.target as HTMLElement).closest("button")) return;
    osWindow.startDragging();
  }

  return (
    <div
      onMouseDown={onCardMouseDown}
      className={cn(
        "absolute inset-1 overflow-hidden rounded-[18px] border border-white/10 bg-black transition-all duration-300 ease-out",
        visible
          ? "translate-x-0 scale-100 opacity-100"
          : "pointer-events-none translate-x-7 scale-[0.98] opacity-0"
      )}
    >
      <video
        ref={videoRef}
        autoPlay
        playsInline
        muted
        className="pointer-events-none absolute inset-0 h-full w-full object-cover"
      />

      <div
        className={cn(
          "pointer-events-none absolute inset-x-0 top-0 flex items-center p-3",
          noLabels
            ? "justify-end"
            : "justify-between bg-gradient-to-b from-black/60 to-transparent"
        )}
      >
        {!noLabels && (
          <span className="text-sm font-semibold text-white drop-shadow">{info?.name}</span>
        )}
        <button
          onClick={onClose}
          aria-label="close"
          className="pointer-events-auto flex h-6 w-6 items-center justify-center rounded-full bg-black/35 text-white/80 backdrop-blur hover:bg-black/60 hover:text-white"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {!noLabels && (info?.badge || info?.detail) && (
        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex flex-col gap-1 bg-gradient-to-t from-black/75 to-transparent p-3">
          {info?.badge && (
            <span className="text-base font-bold text-white drop-shadow">{info.badge}</span>
          )}
          {info?.detail && <span className="text-xs text-white/85 drop-shadow">{info.detail}</span>}
        </div>
      )}
    </div>
  );
}
