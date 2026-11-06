import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type {
  AppConfig,
  MonitorStatus,
  MoveRecord,
  OrganizeProgress,
  PreviewItem,
  ProposedSubject,
  RulesDto,
  TabId,
} from "./types";
import "./App.css";

type TreeNode = {
  name: string;
  files: string[];
  children: Record<string, TreeNode>;
};

function buildPreviewTree(items: PreviewItem[]): TreeNode {
  const root: TreeNode = { name: "Destination", files: [], children: {} };
  for (const item of items) {
    const subject = root.children[item.subject] ?? {
      name: item.subject,
      files: [],
      children: {},
    };
    root.children[item.subject] = subject;
    const typeNode = subject.children[item.fileType] ?? {
      name: item.fileType,
      files: [],
      children: {},
    };
    subject.children[item.fileType] = typeNode;
    typeNode.files.push(item.filename);
  }
  return root;
}

function PreviewTree({ items }: { items: PreviewItem[] }) {
  const tree = buildPreviewTree(items);
  const subjects = Object.values(tree.children).sort((a, b) =>
    a.name.localeCompare(b.name),
  );
  if (subjects.length === 0) {
    return <p className="muted">Nothing selected.</p>;
  }
  return (
    <div className="folder-tree" aria-label="Proposed folder layout">
      <div className="tree-root">Destination/</div>
      {subjects.map((subject) => (
        <div key={subject.name} className="tree-subject">
          <div className="tree-folder">{subject.name}/</div>
          {Object.values(subject.children)
            .sort((a, b) => a.name.localeCompare(b.name))
            .map((typeNode) => (
              <div key={typeNode.name} className="tree-type">
                <div className="tree-folder">{typeNode.name}/</div>
                {typeNode.files.map((file) => (
                  <div key={file} className="tree-file">
                    {file}
                  </div>
                ))}
              </div>
            ))}
        </div>
      ))}
    </div>
  );
}

const IMPORT_PLACEHOLDER = `Paste your courses, syllabus, or folder tree. Examples:

1. Power Systems: load flow, transmission, fault
2. Control Systems - pid, bode, root locus
3. Electrical Machines

Or:
Power Systems/
Control Systems/
Digital Electronics/`;

interface ProposalDraft {
  name: string;
  keywordsText: string;
  enabled: boolean;
}

function toDrafts(list: ProposedSubject[]): ProposalDraft[] {
  return list.map((p) => ({
    name: p.name,
    keywordsText: p.keywords.join(", "),
    enabled: p.enabled,
  }));
}

function fromDrafts(list: ProposalDraft[]): ProposedSubject[] {
  return list
    .filter((d) => d.name.trim())
    .map((d) => ({
      name: d.name.trim(),
      keywords: d.keywordsText
        .split(",")
        .map((k) => k.trim())
        .filter(Boolean),
      enabled: d.enabled,
    }));
}

function UndoIcon() {
  return (
    <svg className="undo-icon" viewBox="0 0 16 16" aria-hidden>
      <path
        d="M5.5 3.5 2.5 6.5l3 3M3 6.5h6.25a3.75 3.75 0 0 1 0 7.5H7"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function shortPath(path: string | null | undefined, max = 48): string {
  if (!path) return "Not set";
  if (path.length <= max) return path;
  return "…" + path.slice(-(max - 1));
}

function formatTime(iso: string): string {
  try {
    return new Date(iso).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso;
  }
}

const ELECTRICAL_PRESET_ID = "btech-electrical";
const ELECTRICAL_SAMPLE = [
  "Circuit Theory",
  "Electrical Machines",
  "Power Systems",
  "Control Systems",
  "Signals and Systems",
  "Digital Electronics",
];
const ONBOARDING_STEPS = ["Welcome", "Folders", "Subjects", "Ready"] as const;

type SubjectChoice = "preset" | "later";

interface OnboardingProps {
  config: AppConfig;
  busy: boolean;
  error: string | null;
  onPickFolder: (field: "watchFolder" | "destinationFolder") => void;
  onLoadPreset: () => Promise<boolean>;
  onFinish: (opts: { startMonitoring: boolean; openImport: boolean }) => void;
}

function Onboarding({
  config,
  busy,
  error,
  onPickFolder,
  onLoadPreset,
  onFinish,
}: OnboardingProps) {
  const [step, setStep] = useState(0);
  const [choice, setChoice] = useState<SubjectChoice>("preset");
  const [presetLoaded, setPresetLoaded] = useState(false);

  const foldersReady = Boolean(config.watchFolder && config.destinationFolder);
  const sameFolder =
    foldersReady && config.watchFolder === config.destinationFolder;

  async function continueFromSubjects() {
    if (choice === "preset" && !presetLoaded) {
      const ok = await onLoadPreset();
      if (!ok) return;
      setPresetLoaded(true);
    }
    setStep(3);
  }

  return (
    <div className="onboarding" role="dialog" aria-modal="true" aria-labelledby="ob-title">
      <div className="ob-sheet">
        <div className="ob-top">
          <ol className="ob-progress" aria-label="Setup progress">
            {ONBOARDING_STEPS.map((label, i) => (
              <li
                key={label}
                className={i < step ? "done" : i === step ? "current" : ""}
                aria-current={i === step ? "step" : undefined}
              >
                <span className="ob-progress-bar" aria-hidden />
                <span className="ob-progress-label">{label}</span>
              </li>
            ))}
          </ol>
          {step < 3 && (
            <button
              type="button"
              className="text-btn ob-skip"
              disabled={busy}
              onClick={() => onFinish({ startMonitoring: false, openImport: false })}
            >
              Skip setup
            </button>
          )}
        </div>

        <div className="ob-body" key={step}>
          {step === 0 && (
            <>
              <h1 id="ob-title" className="ob-wordmark">
                Tidy
                <span className="pulse on" aria-hidden />
              </h1>
              <p className="ob-lede">
                Tidy files new downloads into <strong>Subject / Type</strong> folders.
                It runs entirely on this device and shows you every move before it
                happens.
              </p>
              <ul className="ob-points">
                <li>Watches one folder, waits for downloads to finish</li>
                <li>Matches course subjects by filename and document text</li>
                <li>Nothing moves until you confirm — and batches can be reverted</li>
              </ul>
            </>
          )}

          {step === 1 && (
            <>
              <span className="eyebrow">Step 2 of 4</span>
              <h2 id="ob-title">Where should Tidy look, and where should files go?</h2>
              <div className="ob-folders">
                <div className="ob-folder">
                  <div>
                    <strong>Watch folder</strong>
                    <p className="muted">New files appear here — usually Downloads.</p>
                    <p
                      className={`mono ob-path ${config.watchFolder ? "" : "unset"}`}
                      title={config.watchFolder ?? undefined}
                    >
                      {shortPath(config.watchFolder, 52)}
                    </p>
                  </div>
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => onPickFolder("watchFolder")}
                  >
                    {config.watchFolder ? "Change…" : "Choose…"}
                  </button>
                </div>
                <span className="ob-folder-arrow" aria-hidden />
                <div className="ob-folder">
                  <div>
                    <strong>Destination</strong>
                    <p className="muted">Sorted files land in subfolders here.</p>
                    <p
                      className={`mono ob-path ${config.destinationFolder ? "" : "unset"}`}
                      title={config.destinationFolder ?? undefined}
                    >
                      {shortPath(config.destinationFolder, 52)}
                    </p>
                  </div>
                  <button
                    type="button"
                    className={config.destinationFolder ? "secondary" : "primary-toggle start ob-choose"}
                    onClick={() => onPickFolder("destinationFolder")}
                  >
                    {config.destinationFolder ? "Change…" : "Choose…"}
                  </button>
                </div>
              </div>
              {sameFolder && (
                <p className="ob-warn" role="status">
                  Watch and destination are the same folder. Pick a separate
                  destination so sorted files aren&rsquo;t picked up again.
                </p>
              )}
            </>
          )}

          {step === 2 && (
            <>
              <span className="eyebrow">Step 3 of 4</span>
              <h2 id="ob-title">Which subjects should Tidy sort into?</h2>
              <div className="ob-choices" role="radiogroup" aria-label="Subjects">
                <label className={`ob-choice ${choice === "preset" ? "on" : ""}`}>
                  <input
                    type="radio"
                    name="ob-subjects"
                    checked={choice === "preset"}
                    onChange={() => setChoice("preset")}
                  />
                  <span>
                    <strong>Load B.Tech Electrical preset</strong>
                    <small>13 subjects with keywords, ready to use.</small>
                    <span className="ob-chips" aria-hidden>
                      {ELECTRICAL_SAMPLE.map((s) => (
                        <i key={s}>{s}</i>
                      ))}
                      <i className="more">+7</i>
                    </span>
                  </span>
                </label>
                <label className={`ob-choice ${choice === "later" ? "on" : ""}`}>
                  <input
                    type="radio"
                    name="ob-subjects"
                    checked={choice === "later"}
                    onChange={() => setChoice("later")}
                  />
                  <span>
                    <strong>Import my own later</strong>
                    <small>
                      Keep the general starter subjects for now. Paste a syllabus
                      in the Import tab whenever you&rsquo;re ready.
                    </small>
                  </span>
                </label>
              </div>
            </>
          )}

          {step === 3 && (
            <>
              <span className="eyebrow">All set</span>
              <h2 id="ob-title">Tidy is ready.</h2>
              <p className="ob-lede">
                While monitoring, each finished download is matched to a subject and
                held for review. You&rsquo;ll get a confirm window with the proposed
                folder tree — <strong>nothing moves until you confirm</strong>.
              </p>
              <div className="ob-summary">
                <div>
                  <span className="eyebrow">Watching</span>
                  <p className="mono" title={config.watchFolder ?? undefined}>
                    {shortPath(config.watchFolder, 40)}
                  </p>
                </div>
                <div>
                  <span className="eyebrow">Destination</span>
                  <p className="mono" title={config.destinationFolder ?? undefined}>
                    {shortPath(config.destinationFolder, 40)}
                  </p>
                </div>
                <div>
                  <span className="eyebrow">Subjects</span>
                  <p>
                    {config.subjects.length} subject{config.subjects.length === 1 ? "" : "s"}
                    {presetLoaded ? " · B.Tech Electrical" : ""}
                  </p>
                </div>
              </div>
              <p className="muted">
                Changed your mind? Undo single files or use{" "}
                <em>Revert previous changes</em> on Home.
              </p>
            </>
          )}
        </div>

        {error && (
          <p className="ob-error" role="alert">
            {error}
          </p>
        )}

        <div className="ob-foot">
          {step > 0 && step < 3 ? (
            <button
              type="button"
              className="secondary"
              disabled={busy}
              onClick={() => setStep((s) => s - 1)}
            >
              Back
            </button>
          ) : (
            <span />
          )}

          {step === 0 && (
            <button type="button" className="primary-toggle start" onClick={() => setStep(1)}>
              Get started
            </button>
          )}
          {step === 1 && (
            <div className="ob-next">
              {!foldersReady && <span className="muted">Choose both folders to continue</span>}
              <button
                type="button"
                className="primary-toggle start"
                disabled={!foldersReady || sameFolder}
                onClick={() => setStep(2)}
              >
                Continue
              </button>
            </div>
          )}
          {step === 2 && (
            <button
              type="button"
              className="primary-toggle start"
              disabled={busy}
              onClick={continueFromSubjects}
            >
              {choice === "preset" && !presetLoaded ? "Load preset & continue" : "Continue"}
            </button>
          )}
          {step === 3 && (
            <div className="ob-next">
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() =>
                  onFinish({ startMonitoring: false, openImport: choice === "later" })
                }
              >
                {choice === "later" ? "Import subjects first" : "Not yet"}
              </button>
              <button
                type="button"
                className="primary-toggle start"
                disabled={busy}
                onClick={() => onFinish({ startMonitoring: true, openImport: false })}
              >
                Start monitoring
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default function App() {
  const [tab, setTab] = useState<TabId>("home");
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<MonitorStatus | null>(null);
  const [history, setHistory] = useState<MoveRecord[]>([]);
  const [rules, setRules] = useState<RulesDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<OrganizeProgress | null>(null);
  const [preview, setPreview] = useState<PreviewItem[] | null>(null);
  const [previewKind, setPreviewKind] = useState<"existing" | "pending">("existing");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [canRevert, setCanRevert] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [draftKeywords, setDraftKeywords] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [editKeywords, setEditKeywords] = useState("");
  const [importText, setImportText] = useState("");
  const [importImageUrl, setImportImageUrl] = useState<string | null>(null);
  const [importImageName, setImportImageName] = useState<string | null>(null);
  const [proposals, setProposals] = useState<ProposalDraft[] | null>(null);
  const [proposedFrom, setProposedFrom] = useState("");
  const [importMode, setImportMode] = useState<"replace" | "merge">("replace");

  const refresh = useCallback(async () => {
    try {
      const [cfg, mon, hist, revert] = await Promise.all([
        invoke<AppConfig>("get_config"),
        invoke<MonitorStatus>("get_monitor_status"),
        invoke<MoveRecord[]>("get_history", { limit: 40 }),
        invoke<boolean>("can_revert_previous"),
      ]);
      setConfig(cfg);
      setStatus(mon);
      setHistory(hist);
      setCanRevert(revert);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const refreshRules = useCallback(async () => {
    try {
      setRules(await invoke<RulesDto>("get_rules"));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => {
    if (tab === "rules") refreshRules();
  }, [tab, refreshRules]);

  useEffect(() => {
    const unsubs: Array<() => void> = [];
    let cancelled = false;

    (async () => {
      const u1 = await listen<MoveRecord>("file-organized", (ev) => {
        setHistory((h) => [ev.payload, ...h.filter((r) => r.id !== ev.payload.id)].slice(0, 40));
      });
      const u2 = await listen<MonitorStatus>("monitor-status", (ev) => {
        setStatus(ev.payload);
      });
      const u3 = await listen<{ path: string; message: string }>("organize-error", (ev) => {
        setError(`${ev.payload.message}`);
      });
      const u4 = await listen<OrganizeProgress>("organize-progress", (ev) => {
        setProgress(ev.payload.phase === "done" ? null : ev.payload);
      });
      const u5 = await listen<PreviewItem[]>("organize-pending", (ev) => {
        const items = ev.payload;
        if (items.length === 0) return;
        setPreviewKind("pending");
        setPreview(items);
        setSelected(new Set(items.map((i) => i.source)));
        setTab("home");
      });
      if (cancelled) {
        u1();
        u2();
        u3();
        u4();
        u5();
        return;
      }
      unsubs.push(u1, u2, u3, u4, u5);
    })();

    return () => {
      cancelled = true;
      unsubs.forEach((u) => u());
    };
  }, []);

  async function toggleMonitor() {
    setError(null);
    setBusy(true);
    try {
      const next = await invoke<MonitorStatus>("toggle_monitoring");
      setStatus(next);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function pickFolder(field: "watchFolder" | "destinationFolder") {
    if (!config) return;
    const selectedPath = await open({ directory: true, multiple: false });
    if (!selectedPath || Array.isArray(selectedPath)) return;
    const next = {
      ...config,
      [field]: selectedPath,
    };
    try {
      const saved = await invoke<AppConfig>("save_config", { newConfig: next });
      setConfig(saved);
      setStatus(await invoke<MonitorStatus>("get_monitor_status"));
    } catch (e) {
      setError(String(e));
    }
  }

  async function updateConfig(patch: Partial<AppConfig>) {
    if (!config) return;
    try {
      const saved = await invoke<AppConfig>("save_config", {
        newConfig: { ...config, ...patch },
      });
      setConfig(saved);
      setStatus(await invoke<MonitorStatus>("get_monitor_status"));
    } catch (e) {
      setError(String(e));
    }
  }

  async function undo(id: string) {
    setError(null);
    try {
      await invoke("undo_move", { id });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function revertPreviousChanges() {
    setError(null);
    setBusy(true);
    try {
      await invoke("revert_previous_changes");
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function runImportPropose() {
    setError(null);
    setBusy(true);
    try {
      const next = await invoke<ProposedSubject[]>("propose_subjects_from_text", {
        text: importText,
      });
      setProposals(toDrafts(next));
      setProposedFrom(importText);
    } catch (e) {
      setError(String(e));
      setProposals(null);
    } finally {
      setBusy(false);
    }
  }

  function updateDraft(idx: number, patch: Partial<ProposalDraft>) {
    setProposals((prev) => prev?.map((d, i) => (i === idx ? { ...d, ...patch } : d)) ?? null);
  }

  async function confirmImport() {
    const subjects = fromDrafts(proposals ?? []);
    if (!subjects.length) return;
    setError(null);
    setBusy(true);
    try {
      const cfg = await invoke<AppConfig>("confirm_proposed_subjects", {
        subjects,
        mode: importMode,
      });
      setConfig(cfg);
      setProposals(null);
      setProposedFrom("");
      setTab("rules");
      await refreshRules();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  function onImportImage(file: File | null) {
    if (importImageUrl) URL.revokeObjectURL(importImageUrl);
    if (!file) {
      setImportImageUrl(null);
      setImportImageName(null);
      return;
    }
    setImportImageName(file.name);
    setImportImageUrl(URL.createObjectURL(file));
  }

  async function onImportTextFile(file: File | null) {
    if (!file) return;
    try {
      const text = await file.text();
      setImportText((prev) => (prev ? `${prev.trim()}\n\n${text}` : text));
    } catch (e) {
      setError(String(e));
    }
  }

  async function openPreview() {
    setError(null);
    setBusy(true);
    setProgress({ phase: "matching", current: 0, total: 0, filename: "" });
    try {
      const items = await invoke<PreviewItem[]>("preview_existing");
      setPreviewKind("existing");
      setPreview(items);
      setSelected(new Set(items.map((i) => i.source)));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  async function applyPreview() {
    if (!preview) return;
    setBusy(true);
    setError(null);
    setProgress({ phase: "moving", current: 0, total: selected.size, filename: "" });
    try {
      const sources = preview.filter((p) => selected.has(p.source)).map((p) => p.source);
      await invoke("apply_existing", { sources });
      setPreview(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  async function dismissPreview() {
    if (previewKind === "pending" && preview) {
      try {
        await invoke("dismiss_pending", {
          sources: preview.map((p) => p.source),
        });
      } catch {
        /* ignore */
      }
    }
    setPreview(null);
  }

  async function toggleSubject(name: string, enabled: boolean) {
    try {
      const cfg = await invoke<AppConfig>("set_subject_enabled", {
        subject: name,
        enabled,
      });
      setConfig(cfg);
      await refreshRules();
    } catch (e) {
      setError(String(e));
    }
  }

  async function applyPreset(presetId: string) {
    setError(null);
    try {
      const cfg = await invoke<AppConfig>("apply_subject_preset", { presetId });
      setConfig(cfg);
      await refreshRules();
    } catch (e) {
      setError(String(e));
    }
  }

  async function saveSubject(name: string, keywordsText: string) {
    setError(null);
    const keywords = keywordsText
      .split(",")
      .map((k) => k.trim())
      .filter(Boolean);
    try {
      const cfg = await invoke<AppConfig>("upsert_subject", { name, keywords });
      setConfig(cfg);
      await refreshRules();
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeSubject(name: string) {
    setError(null);
    try {
      const cfg = await invoke<AppConfig>("delete_subject", { name });
      setConfig(cfg);
      await refreshRules();
    } catch (e) {
      setError(String(e));
    }
  }

  async function loadOnboardingPreset(): Promise<boolean> {
    setError(null);
    setBusy(true);
    try {
      const cfg = await invoke<AppConfig>("apply_subject_preset", {
        presetId: ELECTRICAL_PRESET_ID,
      });
      setConfig(cfg);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function finishOnboarding(opts: { startMonitoring: boolean; openImport: boolean }) {
    if (!config) return;
    setError(null);
    setBusy(true);
    try {
      const saved = await invoke<AppConfig>("save_config", {
        newConfig: { ...config, onboardingComplete: true },
      });
      setConfig(saved);
      setTab(opts.openImport ? "import" : "home");
      const current = await invoke<MonitorStatus>("get_monitor_status");
      setStatus(
        opts.startMonitoring && !current.active
          ? await invoke<MonitorStatus>("start_monitoring")
          : current,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function revealDestination() {
    if (!config?.destinationFolder) return;
    try {
      await revealItemInDir(config.destinationFolder);
    } catch (e) {
      setError(String(e));
    }
  }

  const active = status?.active ?? false;
  const readyCount = proposals ? fromDrafts(proposals).length : 0;

  return (
    <div className="app">
      <header className={`topbar ${tab === "home" ? "on-home" : ""}`}>
        <div className="brand" aria-hidden={tab === "home"}>
          <span className={`brand-mark ${active ? "on" : ""}`} aria-hidden />
          <span className="brand-name">Tidy</span>
        </div>
        <nav className="tabs" aria-label="Main">
          {(
            [
              ["home", "Home"],
              ["import", "Import"],
              ["rules", "Rules"],
              ["settings", "Settings"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              className={tab === id ? "tab active" : "tab"}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </nav>
      </header>

      {error && (
        <div className="banner" role="alert">
          <span>{error}</span>
          <button type="button" className="text-btn" onClick={() => setError(null)}>
            Dismiss
          </button>
        </div>
      )}

      <main className="content">
        {tab === "home" && (
          <section className="home">
            <div className="hero">
              <div className="hero-brand">
                <h1 className="wordmark">
                  Tidy
                  <span className={`pulse ${active ? "on" : "off"}`} aria-hidden />
                </h1>
                <p className="tagline">Downloads, sorted by subject and type.</p>
              </div>
              <div className="hero-status">
                <p className="status-label">
                  <span className="eyebrow">Status</span>
                  {active ? "Organizing" : "Paused"}
                </p>
                <p className="status-msg">{status?.message ?? "Loading…"}</p>
                <button
                  type="button"
                  className={`primary-toggle ${active ? "pause" : "start"}`}
                  onClick={toggleMonitor}
                  disabled={busy}
                >
                  {active ? "Pause" : "Start organizing"}
                </button>
              </div>
            </div>

            <div className={`path-row ${active ? "live" : ""}`}>
              <div className="path-end">
                <span className="eyebrow">Watching</span>
                <p className="mono" title={config?.watchFolder ?? undefined}>
                  {shortPath(config?.watchFolder, 40)}
                </p>
              </div>
              <span className="path-arrow" aria-hidden>
                <span />
              </span>
              <div className="path-end">
                <span className="eyebrow">Destination</span>
                <p className="mono" title={config?.destinationFolder ?? undefined}>
                  {shortPath(config?.destinationFolder, 40)}
                </p>
              </div>
            </div>

            <div className="panel-head">
              <h2>
                Recent activity
                {history.length > 0 && <span className="count">{history.length}</span>}
              </h2>
              <div className="btn-row">
                <button
                  type="button"
                  className="undo-btn last"
                  onClick={revertPreviousChanges}
                  disabled={busy || !canRevert}
                  title={
                    canRevert
                      ? "Move the last confirmed organize batch back to Downloads"
                      : "Nothing to revert yet"
                  }
                >
                  <UndoIcon />
                  Revert previous changes
                </button>
                <button type="button" className="secondary" onClick={openPreview} disabled={busy}>
                  Organize Existing Files…
                </button>
              </div>
            </div>

            <ul className="activity">
              {history.length === 0 && (
                <li className="empty">No moves yet. New downloads will appear here.</li>
              )}
              {history.map((item) => (
                <li key={item.id} className={item.undone ? "undone" : ""}>
                  <time className="activity-time" dateTime={item.timestamp}>
                    {formatTime(item.timestamp)}
                  </time>
                  <div className="activity-main">
                    <strong title={item.filename}>{item.filename}</strong>
                    <span className="route-tag">
                      {item.subject}
                      <i>/</i>
                      {item.fileType}
                    </span>
                  </div>
                  {!item.undone && (
                    <button
                      type="button"
                      className="undo-btn"
                      onClick={() => undo(item.id)}
                      disabled={busy}
                      aria-label={`Undo move of ${item.filename}`}
                    >
                      <UndoIcon />
                      Undo
                    </button>
                  )}
                  {item.undone && <span className="badge">Undone</span>}
                </li>
              ))}
            </ul>
          </section>
        )}

        {tab === "import" && (
          <section className="import-screen">
            <h2>Import subjects</h2>
            <p className="muted intro">
              Paste a course list and Tidy proposes subject rules for you to review —
              parsed on this device, applied only when you confirm.
            </p>

            <ol className="import-steps">
              <li className={`import-step ${proposals ? "done" : "current"}`}>
                <div className="step-head">
                  <span className="step-num" aria-hidden>
                    1
                  </span>
                  <h3>
                    <label htmlFor="import-prompt">Describe your courses</label>
                  </h3>
                  {proposals && <span className="step-state">Proposed</span>}
                </div>

                <div className={`import-grid ${importImageUrl ? "has-ref" : ""}`}>
                  <div className="import-panel">
                    <textarea
                      id="import-prompt"
                      className="import-textarea"
                      value={importText}
                      onChange={(e) => setImportText(e.target.value)}
                      placeholder={IMPORT_PLACEHOLDER}
                      rows={11}
                      spellCheck={false}
                    />
                    <div className="import-actions">
                      <div className="import-attach">
                        <span className="attach-label">Add from file</span>
                        <label className="text-btn file-pill">
                          Text or Markdown
                          <input
                            type="file"
                            accept=".txt,.md,text/plain,text/markdown"
                            hidden
                            onChange={(e) => {
                              onImportTextFile(e.target.files?.[0] ?? null);
                              e.target.value = "";
                            }}
                          />
                        </label>
                        <label className="text-btn file-pill">
                          {importImageUrl ? "Replace image" : "Reference image"}
                          <input
                            type="file"
                            accept="image/*"
                            hidden
                            onChange={(e) => {
                              onImportImage(e.target.files?.[0] ?? null);
                              e.target.value = "";
                            }}
                          />
                        </label>
                      </div>
                      <button
                        type="button"
                        className={`primary-toggle ${proposals ? "pause" : "start"}`}
                        disabled={busy || !importText.trim()}
                        onClick={runImportPropose}
                      >
                        {proposals ? "Propose again" : "Propose rules"}
                      </button>
                    </div>
                  </div>

                  {importImageUrl && (
                    <figure className="import-ref">
                      <img
                        className="import-preview"
                        src={importImageUrl}
                        alt="Course reference you attached"
                      />
                      <figcaption>
                        <span className="import-ref-name mono" title={importImageName ?? undefined}>
                          {importImageName}
                        </span>
                        <button
                          type="button"
                          className="text-btn"
                          onClick={() => onImportImage(null)}
                        >
                          Remove
                        </button>
                        <span className="import-ref-note">
                          For reference only. Tidy reads the text box, not the image.
                        </span>
                      </figcaption>
                    </figure>
                  )}
                </div>
              </li>

              <li className={`import-step ${proposals ? "current" : "pending"}`}>
                <div className="step-head">
                  <span className="step-num" aria-hidden>
                    2
                  </span>
                  <h3>Review and apply</h3>
                  {proposals && proposals.length > 0 && (
                    <span className="step-state">
                      {proposals.length} subject{proposals.length === 1 ? "" : "s"}
                    </span>
                  )}
                </div>

                {!proposals && (
                  <p className="step-placeholder">
                    Proposed subjects appear here. Rename, edit keywords, or drop any
                    before they become rules.
                  </p>
                )}

                {proposals && (
                  <div className="import-confirm">
                    {proposedFrom !== importText && (
                      <p className="import-stale" role="status">
                        The text changed since these were proposed. Propose again to
                        refresh — your edits below will be replaced.
                      </p>
                    )}

                    {proposals.length === 0 ? (
                      <p className="step-placeholder">
                        No subjects recognised. Try one course per line, optionally
                        followed by a colon and keywords.
                      </p>
                    ) : (
                      <div className="proposal-table">
                        <div className="proposal-cols" aria-hidden>
                          <span>Subject</span>
                          <span>Keywords, comma separated</span>
                          <span />
                        </div>
                        <ul className="proposal-list">
                          {proposals.map((p, idx) => (
                            <li key={idx}>
                              <input
                                className="proposal-name"
                                value={p.name}
                                aria-label={`Subject ${idx + 1} name`}
                                placeholder="Subject name"
                                onChange={(e) => updateDraft(idx, { name: e.target.value })}
                              />
                              <input
                                className="proposal-keywords"
                                value={p.keywordsText}
                                aria-label={`Subject ${idx + 1} keywords`}
                                placeholder="No keywords — matches by name only"
                                onChange={(e) =>
                                  updateDraft(idx, { keywordsText: e.target.value })
                                }
                              />
                              <button
                                type="button"
                                className="text-btn danger"
                                aria-label={`Remove ${p.name || "subject"}`}
                                onClick={() =>
                                  setProposals(
                                    (prev) => prev?.filter((_, i) => i !== idx) ?? null,
                                  )
                                }
                              >
                                Remove
                              </button>
                            </li>
                          ))}
                        </ul>
                      </div>
                    )}

                    <button
                      type="button"
                      className="text-btn add-row"
                      onClick={() =>
                        setProposals((prev) => [
                          ...(prev ?? []),
                          { name: "", keywordsText: "", enabled: true },
                        ])
                      }
                    >
                      Add a subject
                    </button>

                    <fieldset className="import-mode">
                      <legend>How to apply</legend>
                      <label className={importMode === "replace" ? "on" : ""}>
                        <input
                          type="radio"
                          name="import-mode"
                          checked={importMode === "replace"}
                          onChange={() => setImportMode("replace")}
                        />
                        <span>
                          <strong>Replace</strong>
                          <small>
                            {config?.subjects.length
                              ? `Your ${config.subjects.length} current subject${
                                  config.subjects.length === 1 ? "" : "s"
                                } will be removed.`
                              : "Use only these subjects."}
                          </small>
                        </span>
                      </label>
                      <label className={importMode === "merge" ? "on" : ""}>
                        <input
                          type="radio"
                          name="import-mode"
                          checked={importMode === "merge"}
                          onChange={() => setImportMode("merge")}
                        />
                        <span>
                          <strong>Merge</strong>
                          <small>Keep current subjects; add new ones and extra keywords.</small>
                        </span>
                      </label>
                    </fieldset>

                    <div className="import-foot">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => {
                          setProposals(null);
                          setProposedFrom("");
                        }}
                      >
                        Discard
                      </button>
                      <button
                        type="button"
                        className="primary-toggle start"
                        disabled={busy || readyCount === 0}
                        onClick={confirmImport}
                      >
                        {importMode === "replace" ? "Replace with" : "Merge in"} {readyCount}{" "}
                        subject{readyCount === 1 ? "" : "s"}
                      </button>
                    </div>
                  </div>
                )}
              </li>
            </ol>
          </section>
        )}

        {tab === "rules" && (
          <section className="rules">
            <h2>File types</h2>
            <p className="muted intro">
              Extensions map to type folders under each subject.
            </p>
            <div className="rule-grid">
              {(rules?.typeRules ?? []).map((rule) => (
                <div key={rule.folder} className="rule-card">
                  <strong>{rule.folder}</strong>
                  <span className="ext">{rule.extensions.map((x) => `.${x}`).join("  ")}</span>
                </div>
              ))}
              <div className="rule-card other">
                <strong>Other</strong>
                <span className="ext">anything unrecognised</span>
              </div>
            </div>

            <h2>Subjects</h2>
            <p className="muted intro">
              Files land in Destination / Subject / Type. Match is by keywords in
              the filename (and document text for live monitoring). Uncertain →
              General.
            </p>

            <div className="preset-row">
              <span className="muted">Load a starter set</span>
              <div className="btn-row">
                {(rules?.presets ?? []).map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className="secondary"
                    onClick={() => applyPreset(p.id)}
                  >
                    {p.label}
                  </button>
                ))}
              </div>
            </div>

            <ul className="subject-list">
              {(rules?.subjects ?? []).map((s) => (
                <li key={s.name}>
                  <div className="subject-main">
                    <div className="subject-row">
                      <strong>{s.name}</strong>
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={s.enabled}
                          onChange={(e) => toggleSubject(s.name, e.target.checked)}
                        />
                        <span>{s.enabled ? "On" : "Off"}</span>
                      </label>
                    </div>
                    {editing === s.name ? (
                      <div className="subject-edit">
                        <input
                          value={editKeywords}
                          onChange={(e) => setEditKeywords(e.target.value)}
                          placeholder="keywords, comma, separated"
                        />
                        <div className="btn-row">
                          <button
                            type="button"
                            className="secondary"
                            onClick={() => {
                              setEditing(null);
                              setEditKeywords("");
                            }}
                          >
                            Cancel
                          </button>
                          <button
                            type="button"
                            className="secondary"
                            onClick={async () => {
                              await saveSubject(s.name, editKeywords);
                              setEditing(null);
                            }}
                          >
                            Save
                          </button>
                        </div>
                      </div>
                    ) : (
                      <p className="muted keywords">
                        {s.keywords.length ? s.keywords.join(" · ") : "No keywords yet"}
                      </p>
                    )}
                    <div className="btn-row">
                      <button
                        type="button"
                        className="text-btn"
                        onClick={() => {
                          setEditing(s.name);
                          setEditKeywords(s.keywords.join(", "));
                        }}
                      >
                        Edit keywords
                      </button>
                      <button
                        type="button"
                        className="text-btn"
                        onClick={() => removeSubject(s.name)}
                      >
                        Remove
                      </button>
                    </div>
                  </div>
                </li>
              ))}
              <li className="subject-general">
                <strong>General</strong>
                <span className="badge">Fallback when unsure</span>
              </li>
            </ul>

            <div className="add-subject">
              <h3>Add subject</h3>
              <input
                value={draftName}
                onChange={(e) => setDraftName(e.target.value)}
                placeholder="e.g. Power Systems"
              />
              <input
                value={draftKeywords}
                onChange={(e) => setDraftKeywords(e.target.value)}
                placeholder="keywords: load flow, transmission, fault"
              />
              <button
                type="button"
                className="secondary"
                disabled={!draftName.trim()}
                onClick={async () => {
                  await saveSubject(draftName.trim(), draftKeywords);
                  setDraftName("");
                  setDraftKeywords("");
                }}
              >
                Add subject
              </button>
            </div>
          </section>
        )}

        {tab === "settings" && config && (
          <section className="settings">
            <h2>Folders</h2>
            <div className="setting-row">
              <div>
                <strong>Watch folder</strong>
                <p className="muted mono" title={config.watchFolder ?? undefined}>
                  {config.watchFolder ?? "Not set"}
                </p>
              </div>
              <button type="button" className="secondary" onClick={() => pickFolder("watchFolder")}>
                Choose…
              </button>
            </div>
            <div className="setting-row">
              <div>
                <strong>Destination folder</strong>
                <p className="muted mono" title={config.destinationFolder ?? undefined}>
                  {config.destinationFolder ?? "Not set"}
                </p>
              </div>
              <div className="btn-row">
                <button
                  type="button"
                  className="secondary"
                  onClick={() => pickFolder("destinationFolder")}
                >
                  Choose…
                </button>
                <button
                  type="button"
                  className="secondary"
                  disabled={!config.destinationFolder}
                  onClick={revealDestination}
                >
                  Reveal
                </button>
              </div>
            </div>

            <h2>Launch</h2>
            <label className="setting-check">
              <input
                type="checkbox"
                checked={config.monitorOnLaunch}
                onChange={(e) => updateConfig({ monitorOnLaunch: e.target.checked })}
              />
              <span>Start monitoring automatically when Tidy launches</span>
            </label>

            <p className="hint">
              Layout: Destination / Subject / Type / filename. Files are never
              overwritten or deleted. Incomplete downloads are ignored until finished.
            </p>
          </section>
        )}
      </main>

      {config && !config.onboardingComplete && (
        <Onboarding
          config={config}
          busy={busy}
          error={error}
          onPickFolder={pickFolder}
          onLoadPreset={loadOnboardingPreset}
          onFinish={finishOnboarding}
        />
      )}

      {progress && (
        <div className="progress-overlay" role="status" aria-live="polite">
          <div className="progress-card">
            <div className="progress-head">
              <div className="progress-spinner" aria-hidden />
              <h2>
                {progress.phase === "moving"
                  ? "Moving files"
                  : "Matching files"}
              </h2>
              <span className="progress-count">
                {progress.total > 0
                  ? `${progress.current} / ${progress.total}`
                  : "Starting…"}
              </span>
            </div>
            {progress.filename && (
              <p className="progress-file" title={progress.filename}>
                {progress.filename}
              </p>
            )}
            <div
              className={`progress-bar ${progress.total > 0 ? "" : "indeterminate"}`}
              aria-hidden
            >
              <div
                className="progress-bar-fill"
                style={{
                  width:
                    progress.total > 0
                      ? `${Math.min(100, (progress.current / progress.total) * 100)}%`
                      : "30%",
                }}
              />
            </div>
            <p className="hint">Please wait — Tidy is working.</p>
          </div>
        </div>
      )}

      {preview && !progress && (
        <div className="modal-backdrop" role="presentation" onClick={dismissPreview}>
          <div
            className="modal preview-modal"
            role="dialog"
            aria-labelledby="preview-title"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-head">
              <h2 id="preview-title">
                {previewKind === "pending"
                  ? "Confirm new downloads"
                  : "Confirm organize"}
              </h2>
              <button type="button" className="text-btn" onClick={dismissPreview}>
                Close
              </button>
            </div>
            {preview.length === 0 ? (
              <p className="muted">No eligible files in the watch folder.</p>
            ) : (
              <>
                <p className="muted intro">
                  Nothing is moved until you confirm. Review the proposed folder
                  layout below.
                </p>
                <div className="preview-layout">
                  <div>
                    <div className="preview-actions">
                      <button
                        type="button"
                        className="text-btn"
                        onClick={() => setSelected(new Set(preview.map((p) => p.source)))}
                      >
                        Select all
                      </button>
                      <button
                        type="button"
                        className="text-btn"
                        onClick={() => setSelected(new Set())}
                      >
                        Select none
                      </button>
                    </div>
                    <ul className="preview-list">
                      {preview.map((item) => (
                        <li key={item.source}>
                          <label>
                            <input
                              type="checkbox"
                              checked={selected.has(item.source)}
                              onChange={(e) => {
                                setSelected((prev) => {
                                  const next = new Set(prev);
                                  if (e.target.checked) next.add(item.source);
                                  else next.delete(item.source);
                                  return next;
                                });
                              }}
                            />
                            <span>
                              <strong>{item.filename}</strong>
                              <span className="route-tag">
                                → {item.subject}/{item.fileType}/
                              </span>
                            </span>
                          </label>
                        </li>
                      ))}
                    </ul>
                  </div>
                  <div className="preview-tree-pane">
                    <p className="eyebrow">How it will look</p>
                    <PreviewTree
                      items={preview.filter((p) => selected.has(p.source))}
                    />
                  </div>
                </div>
                <div className="modal-foot">
                  <button type="button" className="secondary" onClick={dismissPreview}>
                    {previewKind === "pending" ? "Skip for now" : "Cancel"}
                  </button>
                  <button
                    type="button"
                    className="primary-toggle start"
                    disabled={busy || selected.size === 0}
                    onClick={applyPreview}
                  >
                    Confirm & move {selected.size} file
                    {selected.size === 1 ? "" : "s"}
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
