mod ha;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, WebviewUrl,
    WebviewWindowBuilder,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

const MAX_KEEP_VISIBLE: usize = 3;

// Each camera gets its own window with a deterministic, label-safe name.
fn window_label(entity: &str) -> String {
    let safe: String = entity
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("cam-{safe}")
}

// ---------- config / prefs types ----------

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CameraConfig {
    name: String,
    camera_entity: String,
    #[serde(default)]
    motion_entities: Vec<String>,
}

fn default_corner() -> String {
    "top-right".to_string()
}
fn default_margin() -> i32 {
    24
}
fn default_width() -> u32 {
    380
}
fn default_height() -> u32 {
    300
}
fn default_dismiss() -> u64 {
    8
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    #[serde(default)]
    ha_url: String,
    #[serde(default)]
    cloud_url: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    cameras: Vec<CameraConfig>,
    #[serde(default = "default_corner")]
    corner: String,
    #[serde(default = "default_margin")]
    margin: i32,
    #[serde(default = "default_width")]
    width: u32,
    #[serde(default = "default_height")]
    height: u32,
    #[serde(default = "default_dismiss")]
    dismiss_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ha_url: String::new(),
            cloud_url: String::new(),
            token: String::new(),
            cameras: Vec::new(),
            corner: default_corner(),
            margin: default_margin(),
            width: default_width(),
            height: default_height(),
            dismiss_seconds: default_dismiss(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Prefs {
    #[serde(default)]
    cameras: HashMap<String, bool>,
    #[serde(default)]
    keep: HashMap<String, bool>,
    #[serde(default)]
    sound: bool,
    #[serde(default)]
    dismiss_seconds: Option<u64>,
    #[serde(default)]
    show_labels: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnConfig {
    #[serde(default)]
    ha_url: String,
    #[serde(default)]
    cloud_url: String,
    #[serde(default)]
    token: String,
}

#[derive(Clone, PartialEq)]
struct ShownCam {
    entity: String,
    name: String,
    label: String,
    detail: String,
    device_class: Option<String>,
    draggable: bool,
}

#[derive(Clone)]
struct MotionInfo {
    detail: String,
    device_class: Option<String>,
}

struct WebrtcSession {
    sub_id: i64,
    entity: String,
    session_id: Option<String>,
    pending_candidates: Vec<Value>,
}

#[derive(Default)]
struct AppState {
    config: Mutex<Config>,
    prefs: Mutex<Prefs>,
    motion_map: Mutex<HashMap<String, CameraConfig>>,
    assignment: Mutex<HashMap<String, ShownCam>>, // window label -> shown camera
    motion_active: Mutex<HashMap<String, MotionInfo>>, // entity -> live motion
    webrtc: Mutex<HashMap<String, WebrtcSession>>, // window label -> session
    ready: Mutex<HashSet<String>>,                 // window labels whose webview is ready
    dismiss_gen: Mutex<HashMap<String, u64>>,      // entity -> dismiss generation
    tray: Mutex<Option<TrayIcon>>,
    ha_tx: Mutex<Option<UnboundedSender<String>>>,
    ha_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    motion_sub_id: Mutex<Option<i64>>,
    next_id: AtomicI64,
    started: AtomicBool,
}

const DISMISS_OPTIONS: [(&str, u64); 6] = [
    ("3 seconds", 3),
    ("5 seconds", 5),
    ("8 seconds", 8),
    ("15 seconds", 15),
    ("30 seconds", 30),
    ("Until dismissed", 0),
];

const DEVICE_CLASS_LABELS: [(&str, &str); 6] = [
    ("motion", "Motion"),
    ("occupancy", "Occupancy"),
    ("presence", "Presence"),
    ("moving", "Movement"),
    ("sound", "Sound"),
    ("vibration", "Vibration"),
];

fn label_for(device_class: Option<&str>) -> String {
    if let Some(dc) = device_class {
        for (key, label) in DEVICE_CLASS_LABELS {
            if key == dc {
                return label.to_string();
            }
        }
    }
    "Motion".to_string()
}

// ---------- persistence ----------

fn load_config(app: &AppHandle) -> Option<Config> {
    let path = app.path().app_config_dir().ok()?.join("config.json");
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_config(app: &AppHandle, config: &Config) {
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(text) = serde_json::to_string_pretty(config) {
            let _ = std::fs::write(dir.join("config.json"), text);
        }
    }
}

fn save_prefs(app: &AppHandle) {
    let state = app.state::<AppState>();
    let prefs = state.prefs.lock().unwrap().clone();
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(text) = serde_json::to_string_pretty(&prefs) {
            let _ = std::fs::write(dir.join("preferences.json"), text);
        }
    }
}

fn load_or_init_prefs(app: &AppHandle, cfg: &Config) -> Prefs {
    let mut prefs: Prefs = app
        .path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("preferences.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for c in &cfg.cameras {
        prefs.cameras.entry(c.camera_entity.clone()).or_insert(true);
        prefs.keep.entry(c.camera_entity.clone()).or_insert(false);
    }
    if prefs.dismiss_seconds.is_none() {
        prefs.dismiss_seconds = Some(cfg.dismiss_seconds);
    }
    prefs
}

// ---------- helpers ----------

fn targets_from(ha_url: &str, cloud_url: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if !ha_url.trim().is_empty() {
        out.push(("Local".to_string(), ha_url.trim().to_string()));
    }
    if !cloud_url.trim().is_empty() {
        out.push(("Remote".to_string(), cloud_url.trim().to_string()));
    }
    out
}

fn camera_enabled(app: &AppHandle, entity: &str) -> bool {
    let state = app.state::<AppState>();
    let prefs = state.prefs.lock().unwrap();
    *prefs.cameras.get(entity).unwrap_or(&true)
}

fn keep_visible(app: &AppHandle, entity: &str) -> bool {
    let state = app.state::<AppState>();
    let prefs = state.prefs.lock().unwrap();
    *prefs.keep.get(entity).unwrap_or(&false)
}

fn effective_dismiss(app: &AppHandle) -> u64 {
    let state = app.state::<AppState>();
    let prefs = state.prefs.lock().unwrap();
    let cfg = state.config.lock().unwrap();
    prefs.dismiss_seconds.unwrap_or(cfg.dismiss_seconds)
}

fn entity_for_label(app: &AppHandle, label: &str) -> Option<String> {
    let state = app.state::<AppState>();
    let a = state.assignment.lock().unwrap();
    a.get(label).map(|c| c.entity.clone())
}

// ---------- displayed-set reconciliation ----------

// The cameras that should currently be on screen, in a stable order
// (keep-visible first, then motion-active), capped at MAX_OVERLAYS.
fn desired(app: &AppHandle) -> Vec<ShownCam> {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let prefs = state.prefs.lock().unwrap().clone();
    let motion = state.motion_active.lock().unwrap().clone();

    let mut out: Vec<ShownCam> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let push_cam = |out: &mut Vec<ShownCam>,
                    seen: &mut HashSet<String>,
                    c: &CameraConfig,
                    draggable: bool| {
        let mi = motion.get(&c.camera_entity);
        out.push(ShownCam {
            entity: c.camera_entity.clone(),
            name: c.name.clone(),
            label: mi.map(|m| label_for(m.device_class.as_deref())).unwrap_or_default(),
            detail: mi.map(|m| m.detail.clone()).unwrap_or_default(),
            device_class: mi.and_then(|m| m.device_class.clone()),
            draggable,
        });
        seen.insert(c.camera_entity.clone());
    };

    // Keep-visible cameras are capped at MAX_KEEP_VISIBLE.
    let mut keep_count = 0;
    for c in &cfg.cameras {
        if *prefs.keep.get(&c.camera_entity).unwrap_or(&false) && keep_count < MAX_KEEP_VISIBLE {
            push_cam(&mut out, &mut seen, c, true);
            keep_count += 1;
        }
    }
    // Motion-triggered feeds are unlimited.
    for c in &cfg.cameras {
        if !seen.contains(&c.camera_entity) && motion.contains_key(&c.camera_entity) {
            push_cam(&mut out, &mut seen, c, false);
        }
    }
    out
}

fn reconcile(app: &AppHandle) {
    let want = desired(app);
    let want_set: HashSet<String> = want.iter().map(|c| c.entity.clone()).collect();

    let assigned: Vec<String> = {
        let state = app.state::<AppState>();
        let a = state.assignment.lock().unwrap();
        a.values().map(|c| c.entity.clone()).collect()
    };
    for entity in assigned {
        if !want_set.contains(&entity) {
            hide_camera(app, &entity);
        }
    }
    for (i, cam) in want.into_iter().enumerate() {
        show_camera_in(app, cam, i);
    }
}

fn show_camera_in(app: &AppHandle, cam: ShownCam, index: usize) {
    let label = window_label(&cam.entity);
    let prev = {
        let state = app.state::<AppState>();
        let a = state.assignment.lock().unwrap();
        a.get(&label).cloned()
    };
    if prev.as_ref() == Some(&cam) {
        return; // nothing changed
    }
    let updating = prev.is_some(); // labels are per-entity, so prev == same camera
    app.state::<AppState>().assignment.lock().unwrap().insert(label.clone(), cam.clone());

    if updating {
        // Same camera, only labels/badge changed: update without restarting stream.
        if app.state::<AppState>().ready.lock().unwrap().contains(&label) {
            let _ = app.emit_to(
                EventTarget::webview_window(label.clone()),
                "overlay-update",
                overlay_payload(app, &cam),
            );
        }
        return;
    }

    let existed = app.get_webview_window(&label).is_some();
    ensure_window(app, &label);
    // Position fresh windows, and re-cascade motion windows; leave dragged
    // keep-visible windows where the user put them.
    if !existed || !cam.draggable {
        position_overlay(app, &label, index);
    }
    present(app, &label);
}

fn overlay_payload(app: &AppHandle, cam: &ShownCam) -> Value {
    let state = app.state::<AppState>();
    let prefs = state.prefs.lock().unwrap();
    json!({
        "cameraEntity": cam.entity,
        "name": cam.name,
        "label": cam.label,
        "detail": cam.detail,
        "deviceClass": cam.device_class,
        "sound": prefs.sound,
        "showLabels": prefs.show_labels.unwrap_or(true),
        "draggable": cam.draggable,
    })
}

fn present(app: &AppHandle, label: &str) {
    let cam = {
        let state = app.state::<AppState>();
        if !state.ready.lock().unwrap().contains(label) {
            return;
        }
        let a = state.assignment.lock().unwrap();
        a.get(label).cloned()
    };
    if let Some(cam) = cam {
        let _ = app.emit_to(
            EventTarget::webview_window(label.to_string()),
            "overlay-show",
            overlay_payload(app, &cam),
        );
    }
}

fn ensure_window(app: &AppHandle, label: &str) {
    if app.get_webview_window(label).is_some() {
        return; // reuse existing window (preserves any dragged position)
    }
    let cfg = app.state::<AppState>().config.lock().unwrap().clone();
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::App("overlay.html".into()))
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .shadow(false)
        .focused(false)
        .visible(false)
        .inner_size(cfg.width as f64, cfg.height as f64)
        .build();
    if let Err(e) = built {
        eprintln!("failed to create overlay window {label}: {e}");
    }
}

fn position_overlay(app: &AppHandle, label: &str, index: usize) {
    let cfg = app.state::<AppState>().config.lock().unwrap().clone();
    if let Some(win) = app.get_webview_window(label) {
        if let Ok(Some(monitor)) = win.primary_monitor() {
            let scale = monitor.scale_factor();
            let mw = monitor.size().width as f64 / scale;
            let mh = monitor.size().height as f64 / scale;
            let mx = monitor.position().x as f64 / scale;
            let my = monitor.position().y as f64 / scale;
            let w = cfg.width as f64;
            let h = cfg.height as f64;
            let margin = cfg.margin as f64;
            let offset = index as f64 * (h + 12.0);
            let mut x = mx + mw - w - margin;
            let mut y = my + margin + offset;
            if cfg.corner.contains("left") {
                x = mx + margin;
            }
            if cfg.corner.contains("bottom") {
                y = my + mh - h - margin - offset;
            }
            let _ = win.set_size(LogicalSize::new(w, h));
            let _ = win.set_position(LogicalPosition::new(x, y));
        }
    }
}

fn hide_camera(app: &AppHandle, entity: &str) {
    let label = window_label(entity);
    if !app.state::<AppState>().assignment.lock().unwrap().contains_key(&label) {
        return;
    }
    if app.state::<AppState>().ready.lock().unwrap().contains(&label) {
        let _ = app.emit_to(EventTarget::webview_window(label.clone()), "overlay-teardown", ());
    } else {
        finalize_hide(app, &label);
    }
}

fn finalize_hide(app: &AppHandle, label: &str) {
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.hide();
    }
    app.state::<AppState>().assignment.lock().unwrap().remove(label);
    stop_webrtc_for(app, label);
}

// ---------- dismiss timers (per entity) ----------

fn bump_dismiss(app: &AppHandle, entity: &str) -> u64 {
    let state = app.state::<AppState>();
    let mut m = state.dismiss_gen.lock().unwrap();
    let g = m.get(entity).copied().unwrap_or(0) + 1;
    m.insert(entity.to_string(), g);
    g
}

fn schedule_dismiss(app: &AppHandle, entity: &str) {
    let secs = effective_dismiss(app);
    if secs == 0 {
        return; // "until dismissed" — stays until the user closes it
    }
    let my_gen = bump_dismiss(app, entity);
    let app2 = app.clone();
    let entity = entity.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let current = app2
            .state::<AppState>()
            .dismiss_gen
            .lock()
            .unwrap()
            .get(&entity)
            .copied()
            .unwrap_or(0);
        if current == my_gen {
            app2.state::<AppState>().motion_active.lock().unwrap().remove(&entity);
            reconcile(&app2);
        }
    });
}

// ---------- HA message handling ----------

fn handle_motion(app: &AppHandle, event: &Value) {
    let trigger = &event["variables"]["trigger"];
    let entity = trigger["entity_id"].as_str().unwrap_or("").to_string();
    if entity.is_empty() {
        return;
    }
    let to_state = trigger["to_state"]["state"].as_str().unwrap_or("");
    let attrs = &trigger["to_state"]["attributes"];
    let device_class = attrs["device_class"].as_str().map(|s| s.to_string());
    let friendly = attrs["friendly_name"].as_str().unwrap_or("").to_string();

    let camera = {
        let state = app.state::<AppState>();
        let map = state.motion_map.lock().unwrap();
        map.get(&entity).cloned()
    };
    let camera = match camera {
        Some(c) => c,
        None => return,
    };
    if !camera_enabled(app, &camera.camera_entity) {
        return;
    }

    if to_state == "on" {
        app.state::<AppState>().motion_active.lock().unwrap().insert(
            camera.camera_entity.clone(),
            MotionInfo {
                detail: friendly,
                device_class,
            },
        );
        bump_dismiss(app, &camera.camera_entity); // cancel any pending dismiss
        reconcile(app);
    } else if to_state == "off" {
        if keep_visible(app, &camera.camera_entity) {
            // Stays visible; just clear the motion badge.
            app.state::<AppState>()
                .motion_active
                .lock()
                .unwrap()
                .remove(&camera.camera_entity);
            reconcile(app);
        } else {
            schedule_dismiss(app, &camera.camera_entity);
        }
    }
}

fn handle_webrtc_msg(app: &AppHandle, label: &str, event: &Value) {
    match event["type"].as_str().unwrap_or("") {
        "session" => {
            let sid = event["session_id"].as_str().unwrap_or("").to_string();
            let (entity, pending) = {
                let state = app.state::<AppState>();
                let mut guard = state.webrtc.lock().unwrap();
                match guard.get_mut(label) {
                    Some(sess) => {
                        sess.session_id = Some(sid.clone());
                        (sess.entity.clone(), std::mem::take(&mut sess.pending_candidates))
                    }
                    None => return,
                }
            };
            for c in pending {
                send_candidate(app, &entity, &sid, c);
            }
        }
        "answer" => {
            let _ = app.emit_to(
                EventTarget::webview_window(label.to_string()),
                "webrtc-answer",
                json!({ "sdp": event["answer"] }),
            );
        }
        "candidate" => {
            let _ = app.emit_to(
                EventTarget::webview_window(label.to_string()),
                "webrtc-remote-candidate",
                json!({ "candidate": event["candidate"] }),
            );
        }
        "error" => {
            eprintln!("Home Assistant WebRTC error: {}", event["message"]);
            let _ = app.emit_to(
                EventTarget::webview_window(label.to_string()),
                "webrtc-error",
                json!({ "message": event["message"] }),
            );
        }
        _ => {}
    }
}

fn send_raw(app: &AppHandle, msg: String) {
    let state = app.state::<AppState>();
    let tx = state.ha_tx.lock().unwrap();
    if let Some(sender) = tx.as_ref() {
        let _ = sender.send(msg);
    }
}

fn send_candidate(app: &AppHandle, entity: &str, session_id: &str, candidate: Value) {
    let id = app.state::<AppState>().next_id.fetch_add(1, Ordering::SeqCst);
    let msg = json!({
        "id": id,
        "type": "camera/webrtc/candidate",
        "entity_id": entity,
        "session_id": session_id,
        "candidate": candidate,
    })
    .to_string();
    send_raw(app, msg);
}

fn stop_webrtc_for(app: &AppHandle, label: &str) {
    let session = {
        let state = app.state::<AppState>();
        let mut guard = state.webrtc.lock().unwrap();
        guard.remove(label)
    };
    if let Some(sess) = session {
        let id = app.state::<AppState>().next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({
            "id": id,
            "type": "unsubscribe_events",
            "subscription": sess.sub_id,
        })
        .to_string();
        send_raw(app, msg);
    }
}

fn dispatch(app: &AppHandle, text: &str) {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };
    if v["type"] != "event" {
        return;
    }
    let id = v["id"].as_i64().unwrap_or(-1);
    let motion_id = *app.state::<AppState>().motion_sub_id.lock().unwrap();
    if Some(id) == motion_id {
        handle_motion(app, &v["event"]);
        return;
    }
    let label = {
        let state = app.state::<AppState>();
        let w = state.webrtc.lock().unwrap();
        w.iter().find(|(_, s)| s.sub_id == id).map(|(l, _)| l.clone())
    };
    if let Some(label) = label {
        handle_webrtc_msg(app, &label, &v["event"]);
    }
}

// ---------- HA client loop ----------

fn start_ha(app: &AppHandle, motion_entities: Vec<String>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    *app.state::<AppState>().ha_tx.lock().unwrap() = Some(tx);
    let app2 = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        ha_client_loop(app2, rx, motion_entities).await;
    });
    *app.state::<AppState>().ha_task.lock().unwrap() = Some(handle);
}

async fn ha_client_loop(
    app: AppHandle,
    mut rx: UnboundedReceiver<String>,
    motion_entities: Vec<String>,
) {
    loop {
        let (ha_url, cloud_url, token) = {
            let state = app.state::<AppState>();
            let cfg = state.config.lock().unwrap();
            (cfg.ha_url.clone(), cfg.cloud_url.clone(), cfg.token.clone())
        };
        let targets = targets_from(&ha_url, &cloud_url);
        let mut connected = false;
        for (_label, url) in &targets {
            match tokio::time::timeout(Duration::from_secs(10), ha::connect_authed(url, &token))
                .await
            {
                Ok(Ok(ws)) => {
                    connected = true;
                    run_session(&app, ws, &mut rx, &motion_entities).await;
                    break;
                }
                _ => continue,
            }
        }
        tokio::time::sleep(Duration::from_secs(if connected { 2 } else { 5 })).await;
    }
}

async fn run_session(
    app: &AppHandle,
    ws: ha::Ws,
    rx: &mut UnboundedReceiver<String>,
    motion_entities: &[String],
) {
    let id = app.state::<AppState>().next_id.fetch_add(1, Ordering::SeqCst);
    *app.state::<AppState>().motion_sub_id.lock().unwrap() = Some(id);

    let (mut write, mut read) = ws.split();

    if !motion_entities.is_empty() {
        let sub = json!({
            "id": id,
            "type": "subscribe_trigger",
            "trigger": { "platform": "state", "entity_id": motion_entities },
        })
        .to_string();
        if write.send(Message::Text(sub.into())).await.is_err() {
            return;
        }
    }

    // Show the keep-visible cameras now that the connection is live.
    reconcile(app);

    loop {
        tokio::select! {
            incoming = read.next() => {
                match incoming {
                    Some(Ok(Message::Text(t))) => dispatch(app, t.as_str()),
                    Some(Ok(Message::Ping(p))) => { let _ = write.send(Message::Pong(p)).await; }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            out = rx.recv() => {
                match out {
                    Some(s) => { if write.send(Message::Text(s.into())).await.is_err() { break; } }
                    None => break,
                }
            }
        }
    }
}

// ---------- tray menu ----------

fn emit_to_all_shown(app: &AppHandle, event: &str, payload: Value) {
    let labels: Vec<String> = {
        let state = app.state::<AppState>();
        let a = state.assignment.lock().unwrap();
        a.keys().cloned().collect()
    };
    for label in labels {
        let _ = app.emit_to(EventTarget::webview_window(label), event, payload.clone());
    }
}

fn build_and_set_menu(app: &AppHandle) {
    let (cfg, prefs) = {
        let state = app.state::<AppState>();
        let cfg = state.config.lock().unwrap().clone();
        let prefs = state.prefs.lock().unwrap().clone();
        (cfg, prefs)
    };
    let dismiss = prefs.dismiss_seconds.unwrap_or(cfg.dismiss_seconds);

    let menu_result = (|| -> tauri::Result<()> {
        let mut cam_sub = SubmenuBuilder::new(app, "Cameras");
        let mut keep_sub = SubmenuBuilder::new(app, "Keep visible");
        if cfg.cameras.is_empty() {
            cam_sub = cam_sub.item(
                &MenuItemBuilder::with_id("noop", "No cameras configured")
                    .enabled(false)
                    .build(app)?,
            );
            keep_sub = keep_sub.item(
                &MenuItemBuilder::with_id("noop2", "No cameras configured")
                    .enabled(false)
                    .build(app)?,
            );
        } else {
            for c in &cfg.cameras {
                let enabled = *prefs.cameras.get(&c.camera_entity).unwrap_or(&true);
                cam_sub = cam_sub.item(
                    &CheckMenuItemBuilder::with_id(format!("camera:{}", c.camera_entity), &c.name)
                        .checked(enabled)
                        .build(app)?,
                );
                let kept = *prefs.keep.get(&c.camera_entity).unwrap_or(&false);
                keep_sub = keep_sub.item(
                    &CheckMenuItemBuilder::with_id(format!("keep:{}", c.camera_entity), &c.name)
                        .checked(kept)
                        .build(app)?,
                );
            }
        }
        let cam_sub = cam_sub.build()?;
        let keep_sub = keep_sub.build()?;

        let mut dis_sub = SubmenuBuilder::new(app, "Dismiss after");
        for (label, val) in DISMISS_OPTIONS {
            dis_sub = dis_sub.item(
                &CheckMenuItemBuilder::with_id(format!("dismiss:{}", val), label)
                    .checked(dismiss == val)
                    .build(app)?,
            );
        }
        let dis_sub = dis_sub.build()?;

        let sound = CheckMenuItemBuilder::with_id("sound", "Sound")
            .checked(prefs.sound)
            .build(app)?;
        let labels = CheckMenuItemBuilder::with_id("labels", "Show labels")
            .checked(prefs.show_labels.unwrap_or(true))
            .build(app)?;
        let settings = MenuItemBuilder::with_id("settings", "Settings…").build(app)?;
        let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

        let menu = MenuBuilder::new(app)
            .item(&cam_sub)
            .item(&keep_sub)
            .item(&sound)
            .item(&labels)
            .item(&dis_sub)
            .separator()
            .item(&settings)
            .separator()
            .item(&quit)
            .build()?;

        if let Some(tray) = app.state::<AppState>().tray.lock().unwrap().as_ref() {
            tray.set_menu(Some(menu))?;
        }
        Ok(())
    })();

    if let Err(e) = menu_result {
        eprintln!("Failed to build tray menu: {e}");
    }
}

fn set_keep(app: &AppHandle, entity: &str, on: bool) {
    app.state::<AppState>().prefs.lock().unwrap().keep.insert(entity.to_string(), on);
    save_prefs(app);
    build_and_set_menu(app);
    if !on {
        app.state::<AppState>().motion_active.lock().unwrap().remove(entity);
    }
    reconcile(app);
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "settings" => open_setup(app),
        "quit" => app.exit(0),
        "sound" => {
            let sound = {
                let state = app.state::<AppState>();
                let mut prefs = state.prefs.lock().unwrap();
                prefs.sound = !prefs.sound;
                prefs.sound
            };
            save_prefs(app);
            build_and_set_menu(app);
            emit_to_all_shown(app, "overlay-sound", json!({ "sound": sound }));
        }
        "labels" => {
            let show = {
                let state = app.state::<AppState>();
                let mut prefs = state.prefs.lock().unwrap();
                let cur = prefs.show_labels.unwrap_or(true);
                prefs.show_labels = Some(!cur);
                !cur
            };
            save_prefs(app);
            build_and_set_menu(app);
            emit_to_all_shown(app, "overlay-labels", json!({ "show": show }));
        }
        other => {
            if let Some(entity) = other.strip_prefix("camera:") {
                let enabled = {
                    let state = app.state::<AppState>();
                    let mut prefs = state.prefs.lock().unwrap();
                    let cur = *prefs.cameras.get(entity).unwrap_or(&true);
                    prefs.cameras.insert(entity.to_string(), !cur);
                    !cur
                };
                save_prefs(app);
                build_and_set_menu(app);
                if !enabled {
                    app.state::<AppState>().motion_active.lock().unwrap().remove(entity);
                }
                reconcile(app);
            } else if let Some(entity) = other.strip_prefix("keep:") {
                let cur = keep_visible(app, entity);
                set_keep(app, entity, !cur);
            } else if let Some(val) = other.strip_prefix("dismiss:") {
                if let Ok(secs) = val.parse::<u64>() {
                    app.state::<AppState>().prefs.lock().unwrap().dismiss_seconds = Some(secs);
                    save_prefs(app);
                    build_and_set_menu(app);
                }
            }
        }
    }
}

fn create_tray(app: &AppHandle) {
    let mut builder = TrayIconBuilder::new()
        .tooltip("Peek")
        .icon_as_template(true)
        .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()));
    match tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png")) {
        Ok(icon) => builder = builder.icon(icon),
        Err(_) => {
            if let Some(icon) = app.default_window_icon() {
                builder = builder.icon(icon.clone());
            }
        }
    }
    match builder.build(app) {
        Ok(tray) => {
            *app.state::<AppState>().tray.lock().unwrap() = Some(tray);
            build_and_set_menu(app);
        }
        Err(e) => eprintln!("Failed to create tray: {e}"),
    }
}

// ---------- windows / lifecycle ----------

fn open_setup(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("setup") {
        let _ = win.set_focus();
        return;
    }
    if let Err(e) = WebviewWindowBuilder::new(app, "setup", WebviewUrl::App("setup.html".into()))
        .title("Peek Setup")
        .inner_size(720.0, 860.0)
        .min_inner_size(520.0, 600.0)
        .resizable(true)
        .build()
    {
        eprintln!("failed to create setup window: {e}");
    }
}

fn build_motion(cfg: &Config) -> (HashMap<String, CameraConfig>, Vec<String>) {
    let mut map = HashMap::new();
    let mut motion_entities = Vec::new();
    for c in &cfg.cameras {
        for m in &c.motion_entities {
            map.insert(m.clone(), c.clone());
            motion_entities.push(m.clone());
        }
    }
    (map, motion_entities)
}

fn start_app(app: &AppHandle, cfg: Config) {
    let (map, motion_entities) = build_motion(&cfg);
    {
        let state = app.state::<AppState>();
        *state.config.lock().unwrap() = cfg.clone();
        *state.prefs.lock().unwrap() = load_or_init_prefs(app, &cfg);
        *state.motion_map.lock().unwrap() = map;
        state.started.store(true, Ordering::SeqCst);
    }
    create_tray(app);
    start_ha(app, motion_entities);
}

fn reconfigure(app: &AppHandle, cfg: Config) {
    let (map, motion_entities) = build_motion(&cfg);
    {
        let state = app.state::<AppState>();
        *state.config.lock().unwrap() = cfg.clone();
        {
            let mut prefs = state.prefs.lock().unwrap();
            for c in &cfg.cameras {
                prefs.cameras.entry(c.camera_entity.clone()).or_insert(true);
                prefs.keep.entry(c.camera_entity.clone()).or_insert(false);
            }
        }
        *state.motion_map.lock().unwrap() = map;
        state.motion_active.lock().unwrap().clear();
    }
    save_prefs(app);

    // Tear down every overlay window and its state.
    let labels: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with("cam-"))
        .cloned()
        .collect();
    for label in labels {
        stop_webrtc_for(app, &label);
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.close();
        }
    }
    {
        let state = app.state::<AppState>();
        state.assignment.lock().unwrap().clear();
        state.ready.lock().unwrap().clear();
    }

    if let Some(handle) = app.state::<AppState>().ha_task.lock().unwrap().take() {
        handle.abort();
    }
    build_and_set_menu(app);
    start_ha(app, motion_entities);
}

// ---------- commands ----------

#[tauri::command]
fn setup_load(app: AppHandle) -> Option<Config> {
    load_config(&app)
}

#[tauri::command]
async fn setup_test(config: ConnConfig) -> Value {
    let targets = targets_from(&config.ha_url, &config.cloud_url);
    ha::test_auth(targets, config.token).await
}

#[tauri::command]
async fn setup_entities(config: ConnConfig) -> Value {
    let targets = targets_from(&config.ha_url, &config.cloud_url);
    ha::fetch_entities(targets, config.token).await
}

#[tauri::command]
fn setup_save(app: AppHandle, config: Config) -> Value {
    save_config(&app, &config);
    if app.state::<AppState>().started.load(Ordering::SeqCst) {
        reconfigure(&app, config);
    } else {
        start_app(&app, config);
    }
    if let Some(win) = app.get_webview_window("setup") {
        let _ = win.close();
    }
    json!({ "ok": true })
}

#[tauri::command]
fn setup_cancel(app: AppHandle) {
    if let Some(win) = app.get_webview_window("setup") {
        let _ = win.close();
    }
    if !app.state::<AppState>().started.load(Ordering::SeqCst) {
        app.exit(0);
    }
}

#[tauri::command]
fn overlay_ready(app: AppHandle, label: String) {
    app.state::<AppState>().ready.lock().unwrap().insert(label.clone());
    present(&app, &label);
}

#[tauri::command]
fn overlay_present(app: AppHandle, label: String) {
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.show();
    }
}

#[tauri::command]
fn overlay_close(app: AppHandle, label: String) {
    if let Some(entity) = entity_for_label(&app, &label) {
        if keep_visible(&app, &entity) {
            set_keep(&app, &entity, false);
        }
        app.state::<AppState>().motion_active.lock().unwrap().remove(&entity);
        bump_dismiss(&app, &entity);
    }
}

#[tauri::command]
fn overlay_hide(app: AppHandle, label: String) {
    finalize_hide(&app, &label);
    reconcile(&app);
}

#[tauri::command]
fn webrtc_offer(app: AppHandle, label: String, camera_entity: String, sdp: String) {
    stop_webrtc_for(&app, &label);
    let id = app.state::<AppState>().next_id.fetch_add(1, Ordering::SeqCst);
    app.state::<AppState>().webrtc.lock().unwrap().insert(
        label,
        WebrtcSession {
            sub_id: id,
            entity: camera_entity.clone(),
            session_id: None,
            pending_candidates: Vec::new(),
        },
    );
    let msg = json!({
        "id": id,
        "type": "camera/webrtc/offer",
        "entity_id": camera_entity,
        "offer": sdp,
    })
    .to_string();
    send_raw(&app, msg);
}

#[tauri::command]
fn webrtc_candidate(app: AppHandle, label: String, candidate: Value) {
    let (entity, session_id) = {
        let state = app.state::<AppState>();
        let mut guard = state.webrtc.lock().unwrap();
        match guard.get_mut(&label) {
            Some(sess) => match sess.session_id.clone() {
                Some(sid) => (sess.entity.clone(), sid),
                None => {
                    sess.pending_candidates.push(candidate);
                    return;
                }
            },
            None => return,
        }
    };
    send_candidate(&app, &entity, &session_id, candidate);
}

#[tauri::command]
fn webrtc_stop(app: AppHandle, label: String) {
    stop_webrtc_for(&app, &label);
}

// ---------- entry point ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            setup_load,
            setup_test,
            setup_entities,
            setup_save,
            setup_cancel,
            overlay_ready,
            overlay_present,
            overlay_close,
            overlay_hide,
            webrtc_offer,
            webrtc_candidate,
            webrtc_stop,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            match load_config(&handle) {
                Some(cfg) if !cfg.ha_url.is_empty() && !cfg.token.is_empty() => {
                    start_app(&handle, cfg)
                }
                _ => open_setup(&handle),
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Peek is a menu-bar app. Once started, closing the last window
            // (e.g. the setup window) must NOT quit it — only the tray "Quit"
            // (an explicit app.exit, which carries an exit code) should.
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() && app_handle.state::<AppState>().started.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}
