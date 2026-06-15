import { useEffect, useState } from "react";
import { api, type Config, type ConnConfig, type EntityOption } from "@/lib/tauri";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

interface Row {
  name: string;
  cameraEntity: string;
  motionEntities: string[];
}

type StatusKind = "ok" | "err" | "warn" | "busy" | null;

const CORNERS = [
  { value: "top-right", label: "Top right" },
  { value: "top-left", label: "Top left" },
  { value: "bottom-right", label: "Bottom right" },
  { value: "bottom-left", label: "Bottom left" },
];

const STATUS_CLASSES: Record<Exclude<StatusKind, null>, string> = {
  ok: "bg-primary/15 text-primary",
  err: "bg-destructive/15 text-destructive",
  warn: "bg-amber-400/15 text-amber-400",
  busy: "bg-secondary text-muted-foreground",
};

const selectClass =
  "flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring";

function rowsFromConfig(cfg: Config | null): Row[] {
  if (cfg && cfg.cameras?.length) {
    return cfg.cameras.map((c) => ({
      name: c.name,
      cameraEntity: c.cameraEntity,
      motionEntities: c.motionEntities || [],
    }));
  }
  return [{ name: "", cameraEntity: "", motionEntities: [] }];
}

export function Setup() {
  const [haUrl, setHaUrl] = useState("");
  const [cloudUrl, setCloudUrl] = useState("");
  const [token, setToken] = useState("");
  const [corner, setCorner] = useState("top-right");
  const [base, setBase] = useState<Config | null>(null);
  const [cameras, setCameras] = useState<EntityOption[]>([]);
  const [motion, setMotion] = useState<EntityOption[]>([]);
  const [rows, setRows] = useState<Row[] | null>(null);
  const [status, setStatus] = useState<{ kind: StatusKind; text: string }>({
    kind: null,
    text: "",
  });
  const [busy, setBusy] = useState(false);

  const hasConnection = haUrl.trim().length > 0 && token.trim().length > 0;

  async function runLoadEntities(conn: ConnConfig, baseCfg: Config | null) {
    setStatus({ kind: "busy", text: "Loading entities from Home Assistant…" });
    setBusy(true);
    const res = await api.setupEntities(conn);
    setBusy(false);
    if (!res.ok) {
      setStatus({ kind: "err", text: res.error || "Could not load entities." });
      return;
    }
    const cams = res.cameras || [];
    const mot = res.motion || [];
    setCameras(cams);
    setMotion(mot);
    if (!cams.length) {
      setStatus({ kind: "err", text: "No camera entities found in Home Assistant." });
      return;
    }
    setStatus({
      kind: "ok",
      text: `Found ${cams.length} cameras and ${mot.length} binary sensors.`,
    });
    setRows(rowsFromConfig(baseCfg));
  }

  useEffect(() => {
    api.setupLoad().then((cfg) => {
      if (!cfg) return;
      setBase(cfg);
      setHaUrl(cfg.haUrl || "");
      setCloudUrl(cfg.cloudUrl || "");
      setToken(cfg.token || "");
      setCorner(cfg.corner || "top-right");
      if ((cfg.haUrl || "").trim() && (cfg.token || "").trim()) {
        runLoadEntities(
          { haUrl: cfg.haUrl, cloudUrl: cfg.cloudUrl, token: cfg.token },
          cfg
        );
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function onLoadClick() {
    if (!hasConnection) {
      setStatus({ kind: "err", text: "Enter the local URL and access token first." });
      return;
    }
    runLoadEntities(
      { haUrl: haUrl.trim(), cloudUrl: cloudUrl.trim(), token: token.trim() },
      base
    );
  }

  async function onTest() {
    if (!hasConnection) {
      setStatus({ kind: "err", text: "Enter the local URL and access token first." });
      return;
    }
    setStatus({ kind: "busy", text: "Testing local and remote…" });
    setBusy(true);
    const res = await api.setupTest({
      haUrl: haUrl.trim(),
      cloudUrl: cloudUrl.trim(),
      token: token.trim(),
    });
    setBusy(false);
    if (!res.results) {
      setStatus({ kind: "err", text: res.error || "Could not connect." });
      return;
    }
    const text = res.results
      .map((r) => (r.ok ? `${r.label} ✓` : `${r.label} ✗ — ${r.error}`))
      .join("   ·   ");
    const allOk = res.results.every((r) => r.ok);
    const noneOk = res.results.every((r) => !r.ok);
    setStatus({ kind: allOk ? "ok" : noneOk ? "err" : "warn", text });
  }

  async function onSave() {
    if (!hasConnection) {
      setStatus({ kind: "err", text: "Enter the local URL and access token first." });
      return;
    }
    const cams = (rows || [])
      .filter((r) => r.cameraEntity)
      .map((r) => ({
        name: r.name.trim() || r.cameraEntity,
        cameraEntity: r.cameraEntity,
        motionEntities: r.motionEntities,
      }));
    if (!cams.length) {
      setStatus({ kind: "err", text: "Add at least one camera (load cameras, pick an entity)." });
      return;
    }
    const noMotion = cams.find((c) => !c.motionEntities.length);
    if (noMotion) {
      setStatus({ kind: "err", text: `Pick at least one motion sensor for "${noMotion.name}".` });
      return;
    }
    setBusy(true);
    const cfg: Config = {
      haUrl: haUrl.trim(),
      cloudUrl: cloudUrl.trim(),
      token: token.trim(),
      cameras: cams,
      corner,
      margin: base?.margin ?? 24,
      width: base?.width ?? 380,
      height: base?.height ?? 300,
      dismissSeconds: base?.dismissSeconds ?? 8,
    };
    const res = await api.setupSave(cfg);
    if (res && res.ok === false) {
      setBusy(false);
      setStatus({ kind: "err", text: res.error || "Could not save." });
    }
  }

  function updateRow(index: number, patch: Partial<Row>) {
    setRows((prev) =>
      (prev || []).map((r, i) => (i === index ? { ...r, ...patch } : r))
    );
  }

  function addRow() {
    setRows((prev) => [...(prev || []), { name: "", cameraEntity: "", motionEntities: [] }]);
  }

  function removeRow(index: number) {
    setRows((prev) => (prev || []).filter((_, i) => i !== index));
  }

  return (
    <div className="min-h-screen bg-background px-6 py-6 text-foreground">
      <header className="mb-5">
        <h1 className="text-2xl font-semibold tracking-tight">Peek</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Connect to Home Assistant and choose which cameras to watch.
        </p>
      </header>

      <section className="mb-6 flex flex-col gap-3">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Home Assistant
        </h2>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="haUrl">Local URL</Label>
          <Input
            id="haUrl"
            value={haUrl}
            onChange={(e) => setHaUrl(e.target.value)}
            placeholder="http://homeassistant.local:8123"
            autoFocus
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="cloudUrl">
            Remote / Nabu Casa URL (optional, used when local is unreachable)
          </Label>
          <Input
            id="cloudUrl"
            value={cloudUrl}
            onChange={(e) => setCloudUrl(e.target.value)}
            placeholder="https://xxxxxxxx.ui.nabu.casa"
            autoComplete="off"
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="token">Long-lived access token</Label>
          <Input
            id="token"
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="Profile → Security → Long-lived access tokens"
            autoComplete="off"
          />
        </div>
        <div className="flex items-end gap-3">
          <div className="flex flex-1 flex-col gap-1.5">
            <Label htmlFor="corner">Overlay corner</Label>
            <select
              id="corner"
              className={selectClass}
              value={corner}
              onChange={(e) => setCorner(e.target.value)}
            >
              {CORNERS.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
          </div>
          <Button variant="outline" onClick={onLoadClick} disabled={busy}>
            Load cameras
          </Button>
        </div>
      </section>

      <section className="mb-6 flex flex-col gap-3">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Cameras
        </h2>
        {rows === null ? (
          <p className="text-xs text-muted-foreground">
            Load cameras to pick from your Home Assistant entities.
          </p>
        ) : (
          <>
            {rows.map((row, i) => (
              <div
                key={i}
                className="flex flex-col gap-2.5 rounded-lg border border-border bg-white/[0.02] p-3.5"
              >
                <div className="flex flex-col gap-1.5">
                  <Label>Display name</Label>
                  <Input
                    value={row.name}
                    placeholder="Front Door"
                    onChange={(e) => updateRow(i, { name: e.target.value })}
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label>Camera entity</Label>
                  <select
                    className={selectClass}
                    value={row.cameraEntity}
                    onChange={(e) => {
                      const cameraEntity = e.target.value;
                      const match = cameras.find((c) => c.entityId === cameraEntity);
                      updateRow(i, {
                        cameraEntity,
                        name: row.name.trim() ? row.name : match?.name || row.name,
                      });
                    }}
                  >
                    <option value="">— choose a camera —</option>
                    {cameras.map((c) => (
                      <option key={c.entityId} value={c.entityId}>
                        {c.name} ({c.entityId})
                      </option>
                    ))}
                  </select>
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label>Motion sensors</Label>
                  <div className="max-h-48 overflow-y-auto rounded-md border border-input">
                    {motion.length === 0 ? (
                      <p className="px-3 py-2 text-xs text-muted-foreground">
                        No binary sensors found.
                      </p>
                    ) : (
                      motion.map((m) => {
                        const checked = row.motionEntities.includes(m.entityId);
                        return (
                          <label
                            key={m.entityId}
                            className="flex cursor-pointer items-center gap-2 px-3 py-1.5 text-sm hover:bg-secondary"
                          >
                            <input
                              type="checkbox"
                              className="h-3.5 w-3.5 shrink-0 accent-primary"
                              checked={checked}
                              onChange={(e) => {
                                const next = e.target.checked
                                  ? [...row.motionEntities, m.entityId]
                                  : row.motionEntities.filter((id) => id !== m.entityId);
                                updateRow(i, { motionEntities: next });
                              }}
                            />
                            <span className="truncate">
                              {m.name}
                              <span className="text-muted-foreground">
                                {" "}
                                ({m.entityId}
                                {m.deviceClass ? ` · ${m.deviceClass}` : ""})
                              </span>
                            </span>
                          </label>
                        );
                      })
                    )}
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  className="self-start text-destructive hover:text-destructive"
                  onClick={() => removeRow(i)}
                >
                  Remove camera
                </Button>
              </div>
            ))}
            <Button variant="ghost" size="sm" className="self-start" onClick={addRow}>
              + Add another camera
            </Button>
          </>
        )}
      </section>

      {status.kind && (
        <div
          className={cn(
            "mb-4 rounded-md px-3 py-2 text-xs",
            STATUS_CLASSES[status.kind]
          )}
        >
          {status.text}
        </div>
      )}

      <footer className="flex items-center gap-2.5">
        <Button variant="outline" onClick={onTest} disabled={busy}>
          Test connection
        </Button>
        <div className="flex-1" />
        <Button variant="ghost" onClick={() => api.setupCancel()}>
          Cancel
        </Button>
        <Button onClick={onSave} disabled={busy}>
          Save &amp; start
        </Button>
      </footer>
    </div>
  );
}
