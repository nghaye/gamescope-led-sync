import { callBackend, definePlugin } from "@decky/api";
import {
  ButtonItem,
  DropdownField,
  DropdownItem,
  Field,
  ModalRoot,
  PanelSection,
  PanelSectionRow,
  SliderField,
  TextField,
  ToggleField,
  showModal,
} from "decky-frontend-lib";
import { useEffect, useRef, useState } from "react";

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
  capture: { node_name: string };
}

const DEFAULTS: Cfg = {
  wled: { host: null, port: 21324, timeout_s: 2 },
  layout: { top: 132, right: 75, bottom: 132, left: 75, start: "bl", direction: "clockwise", offset: 0 },
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
  capture: { node_name: "gamescope" },
};

const START_OPTIONS: DropdownItem[] = [
  { label: "Bottom-left corner (bl)", data: "bl" },
  { label: "Bottom-center (bc)", data: "bc" },
  { label: "Bottom-right corner (br)", data: "br" },
  { label: "Right-center (rc)", data: "rc" },
  { label: "Top-right corner (tr)", data: "tr" },
  { label: "Top-center (tc)", data: "tc" },
  { label: "Top-left corner (tl)", data: "tl" },
  { label: "Left-center (lc)", data: "lc" },
];

const DIRECTION_OPTIONS: DropdownItem[] = [
  { label: "Clockwise (viewed from the front)", data: "clockwise" },
  { label: "Counterclockwise (viewed from the front)", data: "counterclockwise" },
];

interface ServiceStatus {
  ok: boolean;
  ActiveState: string;
  SubState: string;
  UnitFileState: string;
  enabled: boolean;
}

// WLED segment length the packet always starts at LED index 0 (SPEC §11).
function packetLen(l: Cfg["layout"]): number {
  return l.top + l.right + l.bottom + l.left + Math.max(l.offset, 0);
}

export default definePlugin(() => {
  const [cfg, setCfg] = useState<Cfg>(DEFAULTS);
  const [status, setStatus] = useState("");
  const [service, setService] = useState<ServiceStatus | null>(null);
  const [lFlash, setLFlash] = useState(false);
  const prevL = useRef<number | null>(null);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    callBackend<{ ok: boolean; config?: Cfg }>("get_config").then((res) => {
      if (res.ok && res.config) setCfg({ ...DEFAULTS, ...res.config });
    });
    refreshService();
  }, []);

  const refreshService = () => {
    callBackend<ServiceStatus>("service_status").then(setService);
  };

  // Every control saves on change, debounced 400 ms; no Apply button —
  // the daemon applies the file at the next tick (SPEC §10.2/§7).
  const save = (next: Cfg) => {
    setCfg(next);
    setStatus("saving…");
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(async () => {
      const res = await callBackend<{ ok: boolean; errors?: string[] }>("save_config", { cfg: next });
      setStatus(res.ok ? "Saved ✓" : (res.errors ?? ["invalid config"]).join("\n"));
    }, 400);
  };

  const patch = <S extends keyof Cfg, K extends keyof Cfg[S]>(section: S, key: K, value: Cfg[S][K]) => {
    save({ ...cfg, [section]: { ...cfg[section], [key]: value } } as Cfg);
  };

  // Numeric text field: parse on change, ignore unparsable input (the
  // save-time validation reports the bound).
  const patchNum = <S extends keyof Cfg, K extends keyof Cfg[S]>(section: S, key: K, raw: string) => {
    const n = Number(raw);
    if (raw.trim() !== "" && !Number.isNaN(n)) patch(section, key, n as Cfg[S][K]);
  };

  const sliderNum = (v: unknown): number => {
    if (typeof v === "number") return v;
    const target = (v as { target?: { value?: unknown } })?.target;
    return Number(target?.value);
  };

  // Highlight the packet-length hint whenever L changes (SPEC §10.2/§11).
  const L = packetLen(cfg.layout);
  useEffect(() => {
    if (prevL.current !== null && prevL.current !== L) {
      setLFlash(true);
      const t = setTimeout(() => setLFlash(false), 3000);
      prevL.current = L;
      return () => clearTimeout(t);
    }
    prevL.current = L;
  }, [L]);

  const showLogs = async () => {
    const res = await callBackend<{ ok: boolean; logs: string }>("get_logs", { n: 200 });
    showModal(
      <ModalRoot>
        <pre style={{ maxHeight: "60vh", overflow: "auto", whiteSpace: "pre-wrap" }}>
          {res.logs || "(no logs)"}
        </pre>
      </ModalRoot>,
    );
  };

  const testFrame = async () => {
    const res = await callBackend<{ ok: boolean; error?: string; log?: string }>("test_frame", {});
    if (!res.ok) {
      showModal(
        <ModalRoot>
          <div>{res.error ?? "test frame failed"}</div>
          {res.log ? <pre style={{ whiteSpace: "pre-wrap" }}>{res.log}</pre> : null}
        </ModalRoot>,
      );
      return;
    }
    showModal(
      <ModalRoot>
        <img
          src={`/plugins/gamescope-led-sync/data/last-frame.png?ts=${Date.now()}`}
          alt="last captured frame"
          style={{ maxWidth: "100%" }}
        />
      </ModalRoot>,
    );
  };

  const wled = cfg.wled;
  const layout = cfg.layout;
  const proc = cfg.processing;

  return (
    <>
      <PanelSection title="Service">
        <PanelSectionRow>
          <ToggleField
            label="Enabled at boot"
            checked={service?.enabled ?? false}
            onChange={async (checked: boolean) => {
              await callBackend("service_set_enabled", { enabled: checked });
              refreshService();
            }}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <Field label="Status" description={service ? `${service.ActiveState} (${service.SubState})` : "…"}>
            {service ? `${service.ActiveState} / ${service.SubState}` : "unknown"}
          </Field>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={() => callBackend("service_start", {}).then(refreshService)}>
            Start
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={() => callBackend("service_stop", {}).then(refreshService)}>
            Stop
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={() => callBackend("service_restart", {}).then(refreshService)}>
            Restart
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={showLogs}>
            View logs
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem layout="below" onClick={testFrame}>
            Test one frame
          </ButtonItem>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="WLED">
        <PanelSectionRow>
          <TextField
            label="Host"
            value={wled.host ?? ""}
            onChange={(e) => patch("wled", "host", e.target.value || null)}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <TextField
            label="UDP realtime port (1–65535)"
            value={String(wled.port)}
            onChange={(e) => patchNum("wled", "port", e.target.value)}
          />
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="LED strip">
        <PanelSectionRow>
          <TextField label="Top (0–490)" value={String(layout.top)} onChange={(e) => patchNum("layout", "top", e.target.value)} />
        </PanelSectionRow>
        <PanelSectionRow>
          <TextField label="Right (0–490)" value={String(layout.right)} onChange={(e) => patchNum("layout", "right", e.target.value)} />
        </PanelSectionRow>
        <PanelSectionRow>
          <TextField label="Bottom (0–490)" value={String(layout.bottom)} onChange={(e) => patchNum("layout", "bottom", e.target.value)} />
        </PanelSectionRow>
        <PanelSectionRow>
          <TextField label="Left (0–490)" value={String(layout.left)} onChange={(e) => patchNum("layout", "left", e.target.value)} />
        </PanelSectionRow>
        <PanelSectionRow>
          <DropdownField
            label="Start point"
            rgOptions={START_OPTIONS}
            selectedOption={START_OPTIONS.find((o) => o.data === layout.start) ?? START_OPTIONS[0]}
            onChange={(item) => patch("layout", "start", item.data as string)}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <DropdownField
            label="Direction"
            rgOptions={DIRECTION_OPTIONS}
            selectedOption={DIRECTION_OPTIONS.find((o) => o.data === layout.direction) ?? DIRECTION_OPTIONS[0]}
            onChange={(item) => patch("layout", "direction", item.data as string)}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <SliderField
            label={`Offset (${layout.offset})`}
            min={-30}
            max={30}
            step={1}
            value={layout.offset}
            onChange={(v) => patch("layout", "offset", Math.round(sliderNum(v)))}
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <Field
            label="WLED segment length"
            description={`Set your WLED segment length to L = ${L} (SPEC §11)`}
            style={lFlash ? { color: "#ffd75f" } : undefined}
          >
            L = {L}
          </Field>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Image">
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
            label={`Band (${Math.round(proc.band_frac * 100)} %)`}
            min={0.02}
            max={0.2}
            step={0.01}
            value={proc.band_frac}
            onChange={(v) => patch("processing", "band_frac", sliderNum(v))}
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
        <PanelSectionRow>
          <TextField label="FPS (10–60)" value={String(proc.fps)} onChange={(e) => patchNum("processing", "fps", e.target.value)} />
        </PanelSectionRow>
        <PanelSectionRow>
          <TextField
            label="Advanced: sample height (120–480)"
            value={String(proc.sample_height)}
            onChange={(e) => patchNum("processing", "sample_height", e.target.value)}
          />
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title="Save">
        <PanelSectionRow>
          <Field label="Status" description="Changes save automatically (400 ms debounce); the daemon applies them at the next tick">
            <span style={{ whiteSpace: "pre-wrap" }}>{status}</span>
          </Field>
        </PanelSectionRow>
      </PanelSection>
    </>
  );
});
