import { callable, definePlugin } from "@decky/api";
import manifest from "@decky/manifest";
import {
  ButtonItem,
  type DropdownOption,
  DropdownItem,
  Field,
  PanelSection,
  PanelSectionRow,
  SliderField,
  staticClasses,
  TextField,
  ToggleField,
} from "@decky/ui";
import { useEffect, useRef, useState } from "react";
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

  useEffect(() => {
    callBackend<{ ok: boolean; config?: Cfg }>("get_config").then((res) => {
      if (res.ok && res.config) setCfg({ ...DEFAULTS, ...res.config });
    });
    refreshService();
    refreshCapture();
    refreshWled();
    // Poll the statuses so the main page tracks session start/end and
    // WLED reachability on its own.
    const t = setInterval(() => {
      refreshService();
      refreshCapture();
      refreshWled();
    }, 5000);
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

  // One segment row: a −1 button, the text field, a +1 button.
  const segmentRow = (key: "bottom" | "left" | "top" | "right", label: string) => {
    if (!draft) return null;
    return (
      <PanelSectionRow>
        <div style={{ display: "flex", alignItems: "center", gap: "8px", width: "100%" }}>
          <div style={{ width: "56px", flexShrink: 0 }}>
            <ButtonItem layout="below" onClick={() => stepSegment(key, -1)}>
              −1
            </ButtonItem>
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <TextField
              label={label}
              value={String(draft.layout[key])}
              onChange={(e) => patchDraftNum("layout", key, e.target.value)}
            />
          </div>
          <div style={{ width: "56px", flexShrink: 0 }}>
            <ButtonItem layout="below" onClick={() => stepSegment(key, 1)}>
              +1
            </ButtonItem>
          </div>
        </div>
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

  const mainPage = (
    <>
      <PanelSection title="Status">
        <PanelSectionRow>
          <Field
            label="Service"
            description={
              service
                ? service.error
                  ? `systemctl --user error: ${service.error}`
                  : `enabled at boot: ${service.enabled ? "yes" : "no"}`
                : "…"
            }
          >
            {service ? `${service.ActiveState} / ${service.SubState}` : "unknown"}
          </Field>
        </PanelSectionRow>
        <PanelSectionRow>
          <Field label="Capture" description={capture?.detail || "…"}>
            {capture
              ? capture.state === "capturing"
                ? "capturing"
                : capture.state === "paused"
                  ? "paused"
                  : capture.state === "waiting"
                    ? "waiting for gamescope"
                    : "inactive (service not running)"
              : "unknown"}
          </Field>
        </PanelSectionRow>
        <PanelSectionRow>
          <Field label="FPS" description="live capture rate">
            {capture?.fps != null ? capture.fps.toFixed(1) : "—"}
          </Field>
        </PanelSectionRow>
        <PanelSectionRow>
          <Field
            label="WLED strip"
            description={wled?.configured ? `host ${wled.host}: ${wled.info ?? wled.error ?? ""}` : "no host configured"}
          >
            {wled ? (wled.configured ? (wled.reachable ? "reachable" : "not reachable") : "not configured") : "…"}
          </Field>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Capture">
        <PanelSectionRow>
          <ToggleField
            label="Capture"
            checked={captureOn}
            disabled={!capture?.active}
            onChange={(on) => setCapturePaused(!on)}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <ToggleField
            label="Capture at startup"
            description="saved in the config file; applied immediately on change"
            checked={!cfg.capture.start_paused}
            onChange={(on) => patch("capture", "start_paused", !on)}
          />
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Image">
        <PanelSectionRow>
          <SliderField
            label={`Brightness (${Math.round(proc.brightness * 100)} %)`}
            min={0}
            max={1}
            step={0.01}
            value={proc.brightness}
            onChange={(v) => patch("processing", "brightness", sliderNum(v))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Gamma (${proc.gamma.toFixed(2)})`}
            min={0.5}
            max={2.5}
            step={0.05}
            value={proc.gamma}
            onChange={(v) => patch("processing", "gamma", sliderNum(v))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Saturation (${proc.saturation.toFixed(2)})`}
            min={0}
            max={3}
            step={0.05}
            value={proc.saturation}
            onChange={(v) => patch("processing", "saturation", sliderNum(v))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Smooth (${proc.smooth.toFixed(2)})`}
            min={0.05}
            max={1.0}
            step={0.05}
            value={proc.smooth}
            onChange={(v) => patch("processing", "smooth", sliderNum(v))}
          />
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Configuration">
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={enterConfig}>
            Configuration
          </ButtonItem>
        </PanelSectionRow>
      </PanelSection>
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
        {segmentRow("bottom", "LEDs on the bottom segment")}
        {segmentRow("left", "LEDs on the left segment")}
        {segmentRow("top", "LEDs on the top segment")}
        {segmentRow("right", "LEDs on the right segment")}
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
            min={0.01}
            max={0.2}
            step={0.01}
            value={draft.processing.band_frac}
            onChange={(v) => patchDraft("processing", "band_frac", sliderNum(v))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <TextField
            label="FPS"
            value={String(draft.processing.fps)}
            onChange={(e) => patchDraftNum("processing", "fps", e.target.value)}
          />
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Save">
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
