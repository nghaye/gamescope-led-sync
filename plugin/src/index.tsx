import { callable, definePlugin, toaster } from "@decky/api";
import manifest from "@decky/manifest";
import {
  Button,
  ButtonItem,
  ConfirmModal,
  type DropdownOption,
  DropdownItem,
  Field,
  PanelSection,
  PanelSectionRow,
  showModal,
  SliderField,
  staticClasses,
  TextField,
  ToggleField,
} from "@decky/ui";
import { type CSSProperties, useEffect, useRef, useState } from "react";
import { FaLightbulb } from "react-icons/fa";

// Decky Loader 3.x removed callBackend: with api_version 1, backend methods
// are called with positional args via the loader/call_plugin_method route.
const callBackend = <T,>(method: string, ...args: unknown[]): Promise<T> =>
  (callable(method) as (...a: unknown[]) => Promise<T>)(...args);

// The config file schema (SPEC §4.2); CLI-only flags never appear here.
interface Cfg {
  wled: { host: string | null; port: number; timeout_s: number };
  layout: {
    top: number;
    right: number;
    bottom: number;
    left: number;
    start: string;
    direction: string;
    offset: number;
    gap: number;
  };
  processing: {
    fps: number;
    sample_height: number;
    band_frac: number;
    saturation: number;
    gamma: number;
    smooth: number;
    brightness: number;
  };
  behavior: { on_no_session: string };
  capture: { node_name: string; start_paused: boolean };
}

const DEFAULTS: Cfg = {
  wled: { host: null, port: 21324, timeout_s: 2 },
  layout: { top: 132, right: 75, bottom: 132, left: 75, start: "bl", direction: "clockwise", offset: 0, gap: 0 },
  processing: {
    fps: 30,
    sample_height: 240,
    band_frac: 0.08,
    saturation: 1.0,
    gamma: 1.0,
    smooth: 0.5,
    brightness: 1.0,
  },
  behavior: { on_no_session: "preset" },
  capture: { node_name: "gamescope", start_paused: false },
};

// Full-word labels only — no bl/bc/... codes in the UI.
const START_OPTIONS: DropdownOption[] = [
  { label: "Bottom-left corner", data: "bl" },
  { label: "Bottom-center", data: "bc" },
  { label: "Bottom-right corner", data: "br" },
  { label: "Right-center", data: "rc" },
  { label: "Top-right corner", data: "tr" },
  { label: "Top-center", data: "tc" },
  { label: "Top-left corner", data: "tl" },
  { label: "Left-center", data: "lc" },
];

// Compact numeric input (segment steppers, FPS): same height as the 32px
// stepper buttons.
const NUM_INPUT_STYLE: CSSProperties = {
  width: "64px",
  height: "32px",
  flexShrink: 0,
  boxSizing: "border-box",
  textAlign: "center",
  borderRadius: "4px",
  border: "1px solid rgba(255, 255, 255, 0.25)",
  background: "rgba(0, 0, 0, 0.3)",
  color: "inherit",
  fontSize: "14px",
};

interface CaptureStatus {
  ok: boolean;
  active: boolean;
  state: "capturing" | "waiting" | "paused" | "inactive" | string;
  fps: number | null;
  detail: string;
}

interface ServiceStatus {
  ok: boolean;
  ActiveState: string;
  SubState: string;
  UnitFileState: string;
  enabled: boolean;
  error?: string | null;
}

interface WledStatus {
  ok: boolean;
  configured: boolean;
  host?: string;
  reachable: boolean;
  info?: string | null;
  error?: string | null;
}

interface ProbeResult {
  ok: boolean;
  info?: string;
  error?: string;
}

// install_status (SPEC §11): the whole installation state — the plugin's own
// version, the embedded payload, what is installed (binary/unit/config), the
// service's run/enabled state, and the version match (update_needed is true
// on a mismatch in either direction).
interface InstallStatus {
  ok: boolean;
  plugin_version: string;
  payload_present: boolean;
  binary_installed: boolean;
  binary_version: string | null;
  unit_installed: boolean;
  config_present: boolean;
  ActiveState: string;
  SubState: string;
  active: boolean;
  enabled: boolean;
  versions_match: boolean | null;
  update_needed: boolean;
  error?: string | null;
}

// Return type of service_install / service_uninstall / service_update / start / stop.
interface ActionResult {
  ok: boolean;
  error?: string | null;
  steps?: string[];
  from?: string | null;
  to?: string;
  config_kept?: string | null;
}

const sliderNum = (v: unknown): number => {
  if (typeof v === "number") return v;
  const target = (v as { target?: { value?: unknown } })?.target;
  return Number(target?.value);
};

function Content() {
  const [mode, setMode] = useState<"main" | "config">("main");
  const [cfg, setCfg] = useState<Cfg>(DEFAULTS);
  const [draft, setDraft] = useState<Cfg | null>(null);
  const [service, setService] = useState<ServiceStatus | null>(null);
  const [capture, setCapture] = useState<CaptureStatus | null>(null);
  const [wled, setWled] = useState<WledStatus | null>(null);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [install, setInstall] = useState<InstallStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [installMsg, setInstallMsg] = useState("");
  const [configStatus, setConfigStatus] = useState("");
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const previewTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const probeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // True when entering the configuration screen paused the capture
  // ourselves, so only we resume it on exit.
  const pausedByUs = useRef(false);

  const refreshService = () => {
    callBackend<ServiceStatus>("service_status").then(setService);
  };
  const refreshCapture = () => {
    callBackend<CaptureStatus>("capture_status").then(setCapture);
  };
  const refreshWled = () => {
    callBackend<WledStatus>("wled_status").then(setWled);
  };
  const refreshInstall = () => {
    callBackend<InstallStatus>("install_status").then(setInstall);
  };
  const refreshAll = () => {
    refreshService();
    refreshCapture();
    refreshWled();
    refreshInstall();
  };

  useEffect(() => {
    callBackend<{ ok: boolean; config?: Cfg }>("get_config").then((res) => {
      if (res.ok && res.config) setCfg({ ...DEFAULTS, ...res.config });
    });
    refreshAll();
    // Poll the statuses so the main page tracks session start/end, WLED
    // reachability and the installation state on its own.
    const t = setInterval(refreshAll, 5000);
    return () => clearInterval(t);
  }, []);

  // Main page: every control saves on change, debounced 400 ms; the
  // daemon applies the file at the next tick, so the effect is immediate
  // and the value is reloaded at boot (SPEC §10.2/§7).
  const save = (next: Cfg) => {
    setCfg(next);
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      callBackend("save_config", next);
    }, 400);
  };

  const patch = <S extends keyof Cfg, K extends keyof Cfg[S]>(section: S, key: K, value: Cfg[S][K]) => {
    save({ ...cfg, [section]: { ...cfg[section], [key]: value } } as Cfg);
  };

  // Capture toggle on the main page: SIGUSR1 pause/resume (runtime only;
  // persistence at boot is the «Capture at startup» switch).
  const setCapturePaused = async (paused: boolean) => {
    await callBackend("capture_set_paused", paused);
    refreshCapture();
    refreshService();
  };

  // --- installation section (SPEC §11) ---------------------------------------

  const notify = (ok: boolean, label: string, body: string) =>
    toaster.toast({ title: `Gamescope LED Sync — ${label}`, body, critical: !ok });

  // Run a backend install action (install / update / uninstall) with the busy
  // state, the on-page error line and a toast; then refresh every status.
  const runAction = async (
    method: "service_install" | "service_uninstall" | "service_update",
    label: string,
  ) => {
    setBusy(true);
    setInstallMsg("");
    try {
      const res = await callBackend<ActionResult>(method);
      if (res.ok) {
        notify(true, label, (res.steps ?? []).join("\n") || "done");
      } else {
        setInstallMsg(res.error ?? `${label} failed`);
        notify(false, label, res.error ?? `${label} failed`);
      }
    } catch (e) {
      setInstallMsg(String(e));
      notify(false, label, String(e));
    } finally {
      setBusy(false);
      refreshAll();
    }
  };

  const startService = async () => {
    setBusy(true);
    try {
      const res = await callBackend<ActionResult>("service_start");
      if (!res.ok) {
        setInstallMsg(res.error ?? "start failed");
        notify(false, "Start", res.error ?? "start failed");
      }
    } finally {
      setBusy(false);
      refreshAll();
    }
  };

  const stopService = async () => {
    setBusy(true);
    try {
      const res = await callBackend<ActionResult>("service_stop");
      if (!res.ok) {
        setInstallMsg(res.error ?? "stop failed");
        notify(false, "Stop", res.error ?? "stop failed");
      }
    } finally {
      setBusy(false);
      refreshAll();
    }
  };

  // Uninstall asks for confirmation first; the config file is kept.
  const askUninstall = () => {
    showModal(
      <ConfirmModal
        strTitle="Uninstall the service and the binary?"
        strDescription="Stops and disables gamescope-led-sync, then removes ~/.local/bin/gamescope-led-sync and the systemd user unit. The config file ~/.config/gamescope-led-sync/config.toml is kept."
        strOKButtonText="Uninstall"
        strCancelButtonText="Cancel"
        bDestructiveWarning
        onOK={() => runAction("service_uninstall", "Uninstall")}
      />,
    );
  };

  // --- configuration screen -------------------------------------------------

  // Immediate strip preview for a draft change: respawn the backend's
  // --preview-strip process with the draft layout (debounced so typing a
  // LED count does not respawn per keystroke). Nothing is written to the
  // config file here.
  const previewDraft = (next: Cfg) => {
    if (previewTimer.current) clearTimeout(previewTimer.current);
    previewTimer.current = setTimeout(() => {
      callBackend<{ ok: boolean; errors?: string[] }>(
        "preview_set",
        next.wled.host ?? "",
        next.wled.port,
        next.wled.timeout_s,
        next.layout,
      ).then((res) => {
        if (!res.ok) setConfigStatus((res.errors ?? ["preview failed"]).join("\n"));
        else setConfigStatus("");
      });
    }, 300);
  };

  const probeHost = (host: string | null) => {
    if (probeTimer.current) clearTimeout(probeTimer.current);
    probeTimer.current = setTimeout(() => {
      callBackend<ProbeResult>("probe_wled", host ?? "").then(setProbe);
    }, 600);
  };

  const enterConfig = async () => {
    // The preview owns the strip while configuring: pause the capture if
    // it is running (and remember it, to resume on exit).
    const st = await callBackend<CaptureStatus>("capture_status");
    pausedByUs.current = !!st.active && st.state !== "paused";
    if (pausedByUs.current) await callBackend("capture_set_paused", true);
    const d = JSON.parse(JSON.stringify(cfg)) as Cfg;
    setDraft(d);
    setConfigStatus("");
    setProbe(null);
    setMode("config");
    await callBackend("preview_set", d.wled.host ?? "", d.wled.port, d.wled.timeout_s, d.layout);
    probeHost(d.wled.host);
    refreshCapture();
  };

  const updateDraft = (next: Cfg, hostChanged: boolean) => {
    setDraft(next);
    previewDraft(next);
    if (hostChanged) probeHost(next.wled.host);
  };

  const patchDraft = <S extends keyof Cfg, K extends keyof Cfg[S]>(section: S, key: K, value: Cfg[S][K]) => {
    if (!draft) return;
    updateDraft({ ...draft, [section]: { ...draft[section], [key]: value } } as Cfg, false);
  };

  const patchDraftNum = <S extends keyof Cfg, K extends keyof Cfg[S]>(
    section: S,
    key: K,
    raw: string,
  ) => {
    const n = Number(raw);
    if (raw.trim() !== "" && !Number.isNaN(n)) patchDraft(section, key, n as Cfg[S][K]);
  };

  // Step one segment's LED count by ±1 with the on-screen buttons, so fine
  // adjustments need no keyboard. Clamped at the §4.3 bounds (each edge
  // 0..=490, count 1..=490): at a bound the button does nothing.
  const stepSegment = (key: "bottom" | "left" | "top" | "right", delta: 1 | -1) => {
    if (!draft) return;
    const cur = draft.layout[key];
    const next = cur + delta;
    if (next < 0 || next > 490) return;
    const count = draft.layout.top + draft.layout.right + draft.layout.bottom + draft.layout.left;
    const nextCount = count - cur + next;
    if (nextCount < 1 || nextCount > 490) return;
    patchDraft("layout", key, next);
  };

  // One segment row: a compact −1 button, a narrow number field, a compact
  // +1 button — all exactly 32px tall. Dialog-style Button (not the
  // full-width ButtonItem) and a fixed-size native input keep the steppers
  // small; the segment label sits on the row's Field.
  const segmentRow = (key: "bottom" | "left" | "top" | "right", label: string) => {
    if (!draft) return null;
    return (
      <PanelSectionRow>
        <Field label={label}>
          <div
            style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: "8px", width: "100%" }}
          >
            <Button style={{ width: "44px", height: "32px", flexShrink: 0 }} onClick={() => stepSegment(key, -1)}>
              −1
            </Button>
            <input
              type="number"
              value={String(draft.layout[key])}
              onChange={(e) => patchDraftNum("layout", key, e.target.value)}
              style={NUM_INPUT_STYLE}
            />
            <Button style={{ width: "44px", height: "32px", flexShrink: 0 }} onClick={() => stepSegment(key, 1)}>
              +1
            </Button>
          </div>
        </Field>
      </PanelSectionRow>
    );
  };

  const exitConfig = async (saveIt: boolean) => {
    if (previewTimer.current) clearTimeout(previewTimer.current);
    await callBackend("preview_stop");
    if (saveIt && draft) {
      const res = await callBackend<{ ok: boolean; errors?: string[] }>("save_config", draft);
      if (!res.ok) {
        setConfigStatus((res.errors ?? ["invalid config"]).join("\n"));
        return; // stay on the configuration screen
      }
      setCfg(draft);
    }
    if (pausedByUs.current) {
      await callBackend("capture_set_paused", false);
      pausedByUs.current = false;
    }
    setDraft(null);
    setMode("main");
    refreshService();
    refreshCapture();
    refreshWled();
  };

  // --- main page ------------------------------------------------------------

  const proc = cfg.processing;
  const captureOn = !!capture?.active && capture.state !== "paused";

  // Installation state (SPEC §11): installed = binary + unit present. A
  // version mismatch locks every control of the page except «Update» and
  // «Uninstall»; `busy` locks everything while an action runs.
  const installed = !!install && install.binary_installed && install.unit_installed;
  const mismatch = !!install?.update_needed;
  const locked = mismatch || busy;

  // Status tones: ok = green, problem = red, neutral = gray.
  const toneColor = (tone: "ok" | "bad" | "neutral") =>
    tone === "ok" ? "#4caf50" : tone === "bad" ? "#f44336" : "#9e9e9e";

  const serviceLabel = service ? `${service.ActiveState} / ${service.SubState}` : "unknown";
  const serviceTone = !service
    ? "neutral"
    : service.ActiveState === "active"
      ? "ok"
      : service.ActiveState === "failed" || service.ActiveState === "inactive"
        ? "bad"
        : "neutral";

  // The FPS count rides along with the capture state when it is active.
  const captureLabel = !capture
    ? "unknown"
    : capture.state === "capturing"
      ? `active${capture.fps != null ? ` (${Math.round(capture.fps)} fps)` : ""}`
      : capture.state === "paused"
        ? "paused"
        : capture.state === "waiting"
          ? "waiting for gamescope"
          : "inactive (service not running)";
  const captureTone = !capture
    ? "neutral"
    : capture.state === "capturing"
      ? "ok"
      : capture.state === "paused" || capture.state === "waiting"
        ? "neutral"
        : "bad";

  const wledLabel = !wled
    ? "…"
    : wled.configured
      ? wled.reachable
        ? "reachable"
        : "not reachable"
      : "not configured";
  const wledTone = !wled || !wled.configured ? "neutral" : wled.reachable ? "ok" : "bad";

  // Version check: green when aligned, red on a mismatch, gray otherwise.
  const versionTone = !install ? "neutral" : mismatch ? "bad" : installed ? "ok" : "neutral";

  const installSection = (
    <PanelSection title="Installation">
      <PanelSectionRow>
        <Field
          label="Plugin"
          description={install?.payload_present ? "distributed zip — payload embedded" : "dev install — no embedded payload"}
        >
          {install ? `installed — v${install.plugin_version}` : "…"}
        </Field>
      </PanelSectionRow>
      <PanelSectionRow>
        <Field
          label="Service + binary"
          description={
            install?.binary_installed
              ? `unit ${install.unit_installed ? "present" : "missing"}, config ${install.config_present ? "present" : "missing"} — ${install.ActiveState}/${install.SubState}`
              : "not installed"
          }
        >
          {install
            ? install.binary_installed
              ? `installed — v${install.binary_version ?? "unknown"}`
              : "not installed"
            : "…"}
        </Field>
      </PanelSectionRow>
      <PanelSectionRow>
        <Field
          label="Version check"
          description={
            install?.error ??
            (mismatch ? "the installed version differs from the plugin's — update to align them" : "")
          }
        >
          <span style={{ color: toneColor(versionTone) }}>
            {!install
              ? "…"
              : mismatch
                ? `update available: v${install.binary_version ?? "?"} → v${install.plugin_version}`
                : installed
                  ? "up to date"
                  : "not installed"}
          </span>
        </Field>
      </PanelSectionRow>
      {installMsg !== "" && (
        <PanelSectionRow>
          <Field label="Last action">
            <span style={{ whiteSpace: "pre-wrap", color: "#f44336" }}>{installMsg}</span>
          </Field>
        </PanelSectionRow>
      )}
      {!installed && (
        <PanelSectionRow>
          <ButtonItem
            layout="below"
            disabled={busy || !install?.payload_present}
            onClick={() => runAction("service_install", "Install")}
          >
            {busy ? "Working…" : "Install service + binary"}
          </ButtonItem>
        </PanelSectionRow>
      )}
      {installed && !mismatch && (
        <PanelSectionRow>
          <div style={{ display: "flex", gap: "8px", width: "100%" }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              <ButtonItem layout="below" disabled={busy || install.active} onClick={startService}>
                Start
              </ButtonItem>
            </div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <ButtonItem layout="below" disabled={busy || !install.active} onClick={stopService}>
                Stop
              </ButtonItem>
            </div>
          </div>
        </PanelSectionRow>
      )}
      {installed && (
        <PanelSectionRow>
          <ButtonItem layout="below" disabled={busy} onClick={askUninstall}>
            Uninstall…
          </ButtonItem>
        </PanelSectionRow>
      )}
      {installed && (
        <PanelSectionRow>
          <ToggleField
            label="Start at boot"
            description="systemctl --user enable / disable"
            checked={install.enabled}
            disabled={locked}
            onChange={(on) => {
              callBackend("service_set_enabled", on).then(() => refreshInstall());
            }}
          />
        </PanelSectionRow>
      )}
    </PanelSection>
  );

  const mainPage = (
    <>
      {mismatch && (
        <PanelSection title="Update available">
          <PanelSectionRow>
            <ButtonItem layout="below" disabled={busy} onClick={() => runAction("service_update", "Update")}>
              {busy ? "Working…" : `Update v${install?.binary_version ?? "?"} → v${install?.plugin_version ?? "?"}`}
            </ButtonItem>
          </PanelSectionRow>
        </PanelSection>
      )}
      <PanelSection title="Status">
        <PanelSectionRow>
          <Field label="Service">
            <span style={{ color: toneColor(serviceTone) }}>{serviceLabel}</span>
          </Field>
        </PanelSectionRow>
        <PanelSectionRow>
          <Field label="Capture">
            <span style={{ color: toneColor(captureTone) }}>{captureLabel}</span>
          </Field>
        </PanelSectionRow>
        <PanelSectionRow>
          <Field label="WLED strip">
            <span style={{ color: toneColor(wledTone) }}>{wledLabel}</span>
          </Field>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Capture">
        <PanelSectionRow>
          <ToggleField
            label="Capture"
            checked={captureOn}
            disabled={!capture?.active || locked}
            onChange={(on) => setCapturePaused(!on)}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <ToggleField
            label="Capture at startup"
            checked={!cfg.capture.start_paused}
            disabled={locked}
            onChange={(on) => patch("capture", "start_paused", !on)}
          />
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Image">
        <PanelSectionRow>
          <SliderField
            label={`Brightness (${Math.round(proc.brightness * 100)} %)`}
            description="overall output level of the whole strip"
            min={0}
            max={1}
            step={0.01}
            value={proc.brightness}
            disabled={locked}
            onChange={(v) => patch("processing", "brightness", sliderNum(v))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Gamma (${proc.gamma.toFixed(2)})`}
            description="mid-tone curve — above 1.00 lifts dark colors, below 1.00 deepens them"
            min={0.5}
            max={2.5}
            step={0.05}
            value={proc.gamma}
            disabled={locked}
            onChange={(v) => patch("processing", "gamma", sliderNum(v))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Saturation (${proc.saturation.toFixed(2)})`}
            description="color intensity — 1.00 as captured, 0 = grayscale, higher = more vivid"
            min={0}
            max={3}
            step={0.05}
            value={proc.saturation}
            disabled={locked}
            onChange={(v) => patch("processing", "saturation", sliderNum(v))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Smooth (${proc.smooth.toFixed(2)})`}
            description="temporal smoothing — higher slows color changes (less flicker)"
            min={0.05}
            max={1.0}
            step={0.05}
            value={proc.smooth}
            disabled={locked}
            onChange={(v) => patch("processing", "smooth", sliderNum(v))}
          />
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Configuration">
        <PanelSectionRow>
          <ButtonItem layout="below" disabled={locked} onClick={enterConfig}>
            Configure LED Strip
          </ButtonItem>
        </PanelSectionRow>
      </PanelSection>

      {installSection}
    </>
  );

  // --- configuration screen -------------------------------------------------

  const configPage = draft && (
    <>
      <PanelSection title="WLED">
        <PanelSectionRow>
          <TextField
            label="WLED host or IP"
            value={draft.wled.host ?? ""}
            onChange={(e) => {
              const host = e.target.value || null;
              updateDraft({ ...draft, wled: { ...draft.wled, host } }, true);
            }}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <Field label="Host check" description="HTTP GET /json/info on port 80">
            {probe === null ? (
              "…"
            ) : probe.ok ? (
              <span style={{ color: "#4caf50" }}>OK — {probe.info}</span>
            ) : (
              <span style={{ color: "#f44336" }}>NOK — {probe.error}</span>
            )}
          </Field>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="LED strip">
        <PanelSectionRow>
          <DropdownItem
            label="Start point"
            rgOptions={START_OPTIONS}
            selectedOption={START_OPTIONS.find((o) => o.data === draft.layout.start) ?? START_OPTIONS[0]}
            onChange={(item) => patchDraft("layout", "start", item.data as string)}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <ToggleField
            label="Clockwise rotation"
            description="off = counterclockwise (viewed from the front)"
            checked={draft.layout.direction === "clockwise"}
            onChange={(on) => patchDraft("layout", "direction", on ? "clockwise" : "counterclockwise")}
          />
        </PanelSectionRow>
        {segmentRow("bottom", "Bottom Segment")}
        {segmentRow("left", "Left Segment")}
        {segmentRow("top", "Top Segment")}
        {segmentRow("right", "Right Segment")}
        <PanelSectionRow>
          <Field
            label="Total active LEDs"
            description={`${draft.layout.top} + ${draft.layout.right} + ${draft.layout.bottom} + ${draft.layout.left}`}
          >
            {draft.layout.top + draft.layout.right + draft.layout.bottom + draft.layout.left}
          </Field>
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Offset (${draft.layout.offset})`}
            description="Circular shift of the whole strip (the last LED is next to the first): + shifts right, − shifts left"
            min={-20}
            max={20}
            step={1}
            value={draft.layout.offset}
            onChange={(v) => patchDraft("layout", "offset", Math.round(sliderNum(v)))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Gap (${draft.layout.gap} LEDs)`}
            description="Hole at the strip's seam, between the last and first LED (e.g. a cable duct at the bottom center): the creux region is skipped when sampling the screen, so each LED shows the zone physically in front of it. The packet length stays the active LED count."
            min={0}
            max={50}
            step={1}
            value={draft.layout.gap}
            onChange={(v) => patchDraft("layout", "gap", Math.round(sliderNum(v)))}
          />
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Capture">
        <PanelSectionRow>
          <SliderField
            label={`Screen band (${Math.round(draft.processing.band_frac * 100)} %)`}
            description="Share of the screen height sampled to color the edges: the top and bottom edges read the top and bottom bands, the left and right edges read the side bands. A higher band averages more screen rows into each LED."
            min={0.01}
            max={0.2}
            step={0.01}
            value={draft.processing.band_frac}
            onChange={(v) => patchDraft("processing", "band_frac", sliderNum(v))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <Field label="FPS" childrenLayout="inline" verticalAlignment="center">
            <input
              type="number"
              value={String(draft.processing.fps)}
              onChange={(e) => patchDraftNum("processing", "fps", e.target.value)}
              style={NUM_INPUT_STYLE}
            />
          </Field>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection>
        {configStatus !== "" && (
          <PanelSectionRow>
            <Field label="Status">
              <span style={{ whiteSpace: "pre-wrap", color: "#f44336" }}>{configStatus}</span>
            </Field>
          </PanelSectionRow>
        )}
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={() => exitConfig(true)}>
            Save and quit
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={() => exitConfig(false)}>
            Cancel
          </ButtonItem>
        </PanelSectionRow>
      </PanelSection>
    </>
  );

  return mode === "config" ? configPage : mainPage;
}

export default definePlugin(() => ({
  name: "Gamescope LED Sync",
  titleView: <div className={staticClasses.Title}>Gamescope LED Sync</div>,
  content: <Content />,
  icon: <FaLightbulb />,
}));
