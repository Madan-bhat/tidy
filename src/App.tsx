import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { ask, open } from "@tauri-apps/plugin-dialog";
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
import Onboarding, { type FinishOptions } from "./Onboarding";
import { FolderIcon, UndoIcon, errText, folderName, formatTime, plural, shortPath } from "./util";
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

function PreviewTree({ items, rootName }: { items: PreviewItem[]; rootName: string }) {
  const tree = buildPreviewTree(items);
  const subjects = Object.values(tree.children).sort((a, b) =>
    a.name.localeCompare(b.name),
  );
  if (subjects.length === 0) {
    return <p className="muted">Tick some files to see where they&rsquo;ll go.</p>;
  }
  return (
    <div className="folder-tree" aria-label="Proposed folder layout">
      <div className="tree-root">{rootName}/</div>
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

const IMPORT_EXAMPLE = `1. Calculus: limits, derivatives, integrals, series
2. Organic Chemistry - alkanes, reaction mechanisms, spectroscopy
3. Data Structures: linked list, binary tree, hashing, graphs
4. World History`;

const IMPORT_PLACEHOLDER = `Paste your courses, syllabus, or folder tree. One course per line works best.

1. Calculus: limits, derivatives, integrals
2. Organic Chemistry - alkanes, reaction mechanisms
3. World History

Or a folder list:
Calculus/
Organic Chemistry/`;

const TABS: ReadonlyArray<readonly [TabId, string]> = [
  ["home", "Home"],
  ["import", "Import"],
  ["rules", "Rules"],
  ["settings", "Settings"],
];

const FILE_MANAGER = /Mac/i.test(navigator.userAgent)
  ? "Finder"
  : /Win/i.test(navigator.userAgent)
    ? "Explorer"
    : "file manager";
const MOD_KEY = /Mac/i.test(navigator.userAgent) ? "⌘" : "Ctrl+";

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
      keywords: splitKeywords(d.keywordsText),
      enabled: d.enabled,
    }));
}

function splitKeywords(text: string): string[] {
  return text
    .split(",")
    .map((k) => k.trim())
    .filter(Boolean);
}

export default function App() {
  const [tab, setTab] = useState<TabId>("home");
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<MonitorStatus | null>(null);
  const [history, setHistory] = useState<MoveRecord[]>([]);
  const [rules, setRules] = useState<RulesDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<OrganizeProgress | null>(null);
  const [preview, setPreview] = useState<PreviewItem[] | null>(null);
  const [previewKind, setPreviewKind] = useState<"existing" | "pending" | "reorganize">(
    "existing",
  );
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [canRevert, setCanRevert] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [draftKeywords, setDraftKeywords] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [editKeywords, setEditKeywords] = useState("");
  const [subjectFilter, setSubjectFilter] = useState("");
  const [importText, setImportText] = useState("");
  const [importImageUrl, setImportImageUrl] = useState<string | null>(null);
  const [importImageName, setImportImageName] = useState<string | null>(null);
  const [proposals, setProposals] = useState<ProposalDraft[] | null>(null);
  const [proposedFrom, setProposedFrom] = useState("");
  const [importMode, setImportMode] = useState<"replace" | "merge">("replace");
  const [version, setVersion] = useState<string | null>(null);
  const [rerun, setRerun] = useState(false);

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
      setError(errText(e));
    }
  }, []);

  const refreshRules = useCallback(async () => {
    try {
      setRules(await invoke<RulesDto>("get_rules"));
    } catch (e) {
      setError(errText(e));
    }
  }, []);

  useEffect(() => {
    refresh();
    getVersion().then(setVersion).catch(() => {});
    // Downloads that finished while the window was closed are still waiting for review.
    invoke<PreviewItem[]>("get_pending")
      .then((items) => {
        if (items.length === 0) return;
        setPreviewKind("pending");
        setPreview(items);
        setSelected(new Set(items.map((i) => i.source)));
      })
      .catch(() => {});
  }, [refresh]);

  useEffect(() => {
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  useEffect(() => {
    if (tab === "rules") refreshRules();
  }, [tab, refreshRules]);

  useEffect(() => {
    if (!notice) return;
    const t = window.setTimeout(() => setNotice(null), 4000);
    return () => window.clearTimeout(t);
  }, [notice]);

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
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  async function pickFolder(field: "watchFolder" | "destinationFolder") {
    if (!config) return;
    const selectedPath = await open({
      directory: true,
      multiple: false,
      canCreateDirectories: true,
      defaultPath: config[field] ?? config.watchFolder ?? undefined,
      title: field === "watchFolder" ? "Choose the folder Tidy should watch" : "Choose where sorted files go",
    });
    if (!selectedPath || Array.isArray(selectedPath)) return;
    try {
      const saved = await invoke<AppConfig>("save_config", {
        newConfig: { ...config, [field]: selectedPath },
      });
      setConfig(saved);
      setStatus(await invoke<MonitorStatus>("get_monitor_status"));
    } catch (e) {
      setError(errText(e));
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
      setError(errText(e));
    }
  }

  async function undo(item: MoveRecord) {
    setError(null);
    try {
      await invoke("undo_move", { id: item.id });
      setNotice(`Moved ${item.filename} back`);
      await refresh();
    } catch (e) {
      setError(errText(e));
    }
  }

  async function revertPreviousChanges() {
    const ok = await ask(
      "Move every file from the most recent batch back to where it came from?",
      { title: "Revert last batch", kind: "warning", okLabel: "Revert", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    setError(null);
    setBusy(true);
    try {
      const undone = await invoke<MoveRecord[]>("revert_previous_changes");
      setNotice(`Moved ${plural(undone.length, "file")} back`);
      await refresh();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  async function revealPath(path: string) {
    try {
      await revealItemInDir(path);
    } catch (e) {
      setError(errText(e));
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
      setError(errText(e));
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
    const existing = config?.subjects.length ?? 0;
    if (importMode === "replace" && existing > 0) {
      const ok = await ask(
        `Your ${plural(existing, "current subject")} will be replaced by ${plural(subjects.length, "new one")}. Files already sorted are not moved.`,
        { title: "Replace subjects?", kind: "warning", okLabel: "Replace", cancelLabel: "Cancel" },
      );
      if (!ok) return;
    }
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
      setNotice(
        importMode === "replace"
          ? `Now sorting into ${plural(subjects.length, "subject")}`
          : `Merged ${plural(subjects.length, "subject")}`,
      );
      await refreshRules();
    } catch (e) {
      setError(errText(e));
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
      setError(errText(e));
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
      setError(errText(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  async function openReorganize() {
    setError(null);
    setBusy(true);
    setProgress({ phase: "matching", current: 0, total: 0, filename: "" });
    try {
      const items = await invoke<PreviewItem[]>("preview_reorganize");
      setPreviewKind("reorganize");
      setPreview(items);
      setSelected(new Set(items.map((i) => i.source)));
    } catch (e) {
      setError(errText(e));
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
      const chosen = preview.filter((p) => selected.has(p.source));
      const sources = chosen.map((p) => p.source);
      const subjects = Object.fromEntries(chosen.map((p) => [p.source, p.subject]));
      const moved = await invoke<MoveRecord[]>("apply_existing", { sources, subjects });
      setPreview(null);
      setNotice(`Filed ${plural(moved.length, "file")} · Undo is available below`);
      await refresh();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  const dismissPreview = useCallback(async () => {
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
  }, [preview, previewKind]);

  async function toggleSubject(name: string, enabled: boolean) {
    try {
      const cfg = await invoke<AppConfig>("set_subject_enabled", {
        subject: name,
        enabled,
      });
      setConfig(cfg);
      await refreshRules();
    } catch (e) {
      setError(errText(e));
    }
  }

  async function applyPreset(presetId: string, label: string) {
    const count = config?.subjects.length ?? 0;
    if (count > 0) {
      const ok = await ask(
        `Replace your ${plural(count, "subject")} with “${label}”? Any keywords you added will be lost. Files already sorted are not moved.`,
        { title: "Load starter set", kind: "warning", okLabel: "Replace", cancelLabel: "Cancel" },
      );
      if (!ok) return;
    }
    setError(null);
    try {
      const cfg = await invoke<AppConfig>("apply_subject_preset", { presetId });
      setConfig(cfg);
      setNotice(`Loaded ${label}`);
      await refreshRules();
    } catch (e) {
      setError(errText(e));
    }
  }

  async function saveSubject(name: string, keywordsText: string): Promise<boolean> {
    setError(null);
    try {
      const cfg = await invoke<AppConfig>("upsert_subject", {
        name,
        keywords: splitKeywords(keywordsText),
      });
      setConfig(cfg);
      setNotice(`Saved ${name}`);
      await refreshRules();
      return true;
    } catch (e) {
      setError(errText(e));
      return false;
    }
  }

  async function removeSubject(name: string) {
    const ok = await ask(
      `Remove “${name}”? New files that would have matched it will go to General instead. Files already sorted stay where they are.`,
      { title: "Remove subject", kind: "warning", okLabel: "Remove", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    setError(null);
    try {
      const cfg = await invoke<AppConfig>("delete_subject", { name });
      setConfig(cfg);
      setNotice(`Removed ${name}`);
      await refreshRules();
    } catch (e) {
      setError(errText(e));
    }
  }

  async function addOnboardingSubjects(text: string): Promise<boolean> {
    setError(null);
    setBusy(true);
    try {
      const subjects = await invoke<ProposedSubject[]>("propose_subjects_from_text", { text });
      const cfg = await invoke<AppConfig>("confirm_proposed_subjects", {
        subjects,
        mode: "merge",
      });
      setConfig(cfg);
      return true;
    } catch (e) {
      setError(errText(e));
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function finishOnboarding(opts: FinishOptions) {
    if (!config) return;
    setError(null);
    setBusy(true);
    try {
      const saved = await invoke<AppConfig>("save_config", {
        newConfig: {
          ...config,
          onboardingComplete: true,
          monitorOnLaunch: opts.monitorOnLaunch,
        },
      });
      setConfig(saved);
      setRerun(false);
      setTab(opts.openImport ? "import" : "home");
      const current = await invoke<MonitorStatus>("get_monitor_status");
      setStatus(
        opts.startMonitoring && !current.active
          ? await invoke<MonitorStatus>("start_monitoring")
          : current,
      );
    } catch (e) {
      setError(errText(e));
      return;
    } finally {
      setBusy(false);
    }
    if (opts.organizeExisting) await openPreview();
  }

  async function rerunSetup() {
    setRerun(true);
    await updateConfig({ onboardingComplete: false });
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape" && preview && !busy) {
        e.preventDefault();
        dismissPreview();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
        const idx = Number(e.key) - 1;
        if (idx >= 0 && idx < TABS.length && config?.onboardingComplete) {
          e.preventDefault();
          setTab(TABS[idx][0]);
        }
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [preview, busy, dismissPreview, config?.onboardingComplete]);

  const active = status?.active ?? false;
  const readyCount = proposals ? fromDrafts(proposals).length : 0;
  const foldersReady = Boolean(config?.watchFolder && config?.destinationFolder);
  const watchName = folderName(config?.watchFolder) || "watch folder";
  const destName = folderName(config?.destinationFolder) || "Destination";

  const visibleSubjects = useMemo(() => {
    const list = rules?.subjects ?? [];
    const q = subjectFilter.trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.keywords.some((k) => k.toLowerCase().includes(q)),
    );
  }, [rules, subjectFilter]);

  const draftExists = Boolean(
    draftName.trim() &&
      rules?.subjects.some((s) => s.name.toLowerCase() === draftName.trim().toLowerCase()),
  );

  const previewStats = useMemo(() => {
    if (!preview) return null;
    const picked = preview.filter((p) => selected.has(p.source));
    return {
      subjects: new Set(picked.map((p) => p.subject)).size,
      unsure: preview.filter((p) => p.subject === "General").length,
    };
  }, [preview, selected]);

  return (
    <div className="app">
      <header className={`topbar ${tab === "home" ? "on-home" : ""}`}>
        <div className="brand" aria-hidden={tab === "home"}>
          <span className={`brand-mark ${active ? "on" : ""}`} aria-hidden />
          <span className="brand-name">Tidy</span>
        </div>
        <nav className="tabs" aria-label="Main">
          {TABS.map(([id, label], i) => (
            <button
              key={id}
              type="button"
              className={tab === id ? "tab active" : "tab"}
              aria-current={tab === id ? "page" : undefined}
              title={`${label} (${MOD_KEY}${i + 1})`}
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
                <p className="tagline">
                  New files in <strong>{watchName}</strong> are filed into{" "}
                  the <strong>{destName}</strong> folder by subject and type — after you confirm.
                </p>
              </div>
              <div className="hero-status">
                <p className="status-label">
                  <span className="eyebrow">Status</span>
                  {!foldersReady ? "Needs setup" : active ? "Watching" : "Paused"}
                </p>
                <p className="status-msg">
                  {!foldersReady
                    ? "Choose a destination folder so Tidy knows where to file things."
                    : (status?.message ?? "Loading…")}
                </p>
                {foldersReady ? (
                  <button
                    type="button"
                    className={`primary-toggle ${active ? "pause" : "start"}`}
                    onClick={toggleMonitor}
                    disabled={busy}
                  >
                    {active ? "Pause watching" : "Start watching"}
                  </button>
                ) : (
                  <button
                    type="button"
                    className="primary-toggle start"
                    onClick={() =>
                      pickFolder(config?.watchFolder ? "destinationFolder" : "watchFolder")
                    }
                  >
                    {config?.watchFolder ? "Choose destination…" : "Choose watch folder…"}
                  </button>
                )}
              </div>
            </div>

            <div className={`path-row ${active ? "live" : ""}`}>
              <button
                type="button"
                className="path-end"
                onClick={() => pickFolder("watchFolder")}
                title="Change watch folder"
              >
                <span className="eyebrow">Watching</span>
                <span className={`mono path-val ${config?.watchFolder ? "" : "unset"}`}>
                  {shortPath(config?.watchFolder, 40)}
                </span>
                <span className="path-change">Change</span>
              </button>
              <span className="path-arrow" aria-hidden>
                <span />
              </span>
              <button
                type="button"
                className="path-end"
                onClick={() => pickFolder("destinationFolder")}
                title="Change destination folder"
              >
                <span className="eyebrow">Filing into</span>
                <span className={`mono path-val ${config?.destinationFolder ? "" : "unset"}`}>
                  {shortPath(config?.destinationFolder, 40)}
                </span>
                <span className="path-change">Change</span>
              </button>
            </div>

            {config && config.subjects.length === 0 && (
              <div className="callout">
                <div>
                  <strong>Add your subjects to get folders like Calculus/Documents</strong>
                  <p className="muted">
                    Without subjects, everything is sorted by type inside General.
                  </p>
                </div>
                <button type="button" className="secondary" onClick={() => setTab("import")}>
                  Import courses…
                </button>
              </div>
            )}

            <div className="quick-actions">
              <button
                type="button"
                className="action-card"
                onClick={openPreview}
                disabled={busy || !foldersReady}
              >
                <strong>Tidy up {watchName} now</strong>
                <span>Sort files that are already there. You review every move first.</span>
              </button>
              <button
                type="button"
                className="action-card"
                onClick={openReorganize}
                disabled={busy || !config?.destinationFolder}
              >
                <strong>Fix mis-sorted files</strong>
                <span>Re-check files in the {destName} folder against your current subjects.</span>
              </button>
              <button
                type="button"
                className="action-card"
                onClick={() => config?.destinationFolder && revealPath(config.destinationFolder)}
                disabled={!config?.destinationFolder}
              >
                <strong>
                  <FolderIcon /> Open {destName}
                </strong>
                <span>Browse your sorted folders in {FILE_MANAGER}.</span>
              </button>
            </div>

            <div className="panel-head">
              <h2>
                Recent activity
                {history.length > 0 && <span className="count">{history.length}</span>}
              </h2>
              <button
                type="button"
                className="undo-btn last"
                onClick={revertPreviousChanges}
                disabled={busy || !canRevert}
                title={
                  canRevert
                    ? "Move every file from the most recent batch back"
                    : "Nothing to revert yet"
                }
              >
                <UndoIcon />
                Revert last batch
              </button>
            </div>

            <ul className="activity">
              {history.length === 0 && (
                <li className="empty">
                  <strong>Nothing filed yet</strong>
                  <span>
                    {active
                      ? `Tidy is watching ${watchName}. When a download finishes you'll be asked to confirm where it goes.`
                      : foldersReady
                        ? "Press Start watching, or tidy up the files already in your watch folder."
                        : "Finish choosing your folders above to get started."}
                  </span>
                </li>
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
                  {!item.undone ? (
                    <div className="activity-btns">
                      <button
                        type="button"
                        className="text-btn"
                        onClick={() => revealPath(item.destination)}
                        aria-label={`Show ${item.filename} in ${FILE_MANAGER}`}
                      >
                        Show
                      </button>
                      <button
                        type="button"
                        className="undo-btn"
                        onClick={() => undo(item)}
                        disabled={busy}
                        aria-label={`Undo move of ${item.filename}`}
                      >
                        <UndoIcon />
                        Undo
                      </button>
                    </div>
                  ) : (
                    <span className="badge">Undone</span>
                  )}
                </li>
              ))}
            </ul>
          </section>
        )}

        {tab === "import" && (
          <section className="import-screen">
            <h2>Import subjects</h2>
            <p className="muted intro">
              Paste your course list or syllabus and Tidy proposes a subject for each course,
              with keywords to match. It&rsquo;s parsed on this device, and nothing changes until
              you review and apply.
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
                  {proposals ? (
                    <span className="step-state">Proposed</span>
                  ) : (
                    !importText && (
                      <button
                        type="button"
                        className="text-btn step-aside"
                        onClick={() => setImportText(IMPORT_EXAMPLE)}
                      >
                        Try an example
                      </button>
                    )
                  )}
                </div>

                <div className={`import-grid ${importImageUrl ? "has-ref" : ""}`}>
                  <div className="import-panel">
                    <textarea
                      id="import-prompt"
                      className="import-textarea"
                      value={importText}
                      onChange={(e) => setImportText(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && importText.trim()) {
                          e.preventDefault();
                          runImportPropose();
                        }
                      }}
                      placeholder={IMPORT_PLACEHOLDER}
                      rows={11}
                      spellCheck={false}
                    />
                    <p className="hint import-tip">
                      Tip: add topics after a colon — <span className="mono">Calculus: limits, derivatives</span>{" "}
                      — so Tidy recognises more of your files.
                    </p>
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
                        title={`${MOD_KEY}Enter`}
                      >
                        {proposals ? "Propose again" : "Propose subjects"}
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
                          For reference while you type. Tidy reads the text box, not the image.
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
                    <span className="step-state">{plural(proposals.length, "subject")}</span>
                  )}
                </div>

                {!proposals && (
                  <p className="step-placeholder">
                    Proposed subjects appear here. Rename them, edit keywords, or remove any
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
                      + Add a subject
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
                              ? `Your ${plural(config.subjects.length, "current subject")} will be removed.`
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
                        {importMode === "replace" ? "Replace with" : "Merge in"}{" "}
                        {plural(readyCount, "subject")}
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
            <div className="section-head">
              <h2>Subjects</h2>
              {(rules?.subjects.length ?? 0) > 6 && (
                <input
                  type="search"
                  className="filter-input"
                  placeholder="Filter subjects or keywords"
                  value={subjectFilter}
                  onChange={(e) => setSubjectFilter(e.target.value)}
                  aria-label="Filter subjects"
                />
              )}
            </div>
            <p className="muted intro">
              Each subject is a folder in {destName}. Tidy compares a file&rsquo;s name — and the
              text inside PDFs, Word and text files — against each subject&rsquo;s keywords. A
              file named <span className="mono">limits_worksheet_3.pdf</span> matches a subject
              with the keyword &ldquo;limits&rdquo;. When nothing is a clear match it goes to{" "}
              <strong>General</strong>.
            </p>

            <div className="preset-row">
              <span className="muted">
                {(rules?.subjects.length ?? 0) === 0
                  ? "No subjects yet — every file goes to General until you add some."
                  : "Have a syllabus? Turn it into subjects in one go."}
              </span>
              <div className="btn-row">
                {(rules?.presets ?? []).map((p) => (
                  <button
                    key={p.id}
                    type="button"
                    className="secondary"
                    onClick={() => applyPreset(p.id, p.label)}
                  >
                    {p.label}
                  </button>
                ))}
                <button type="button" className="secondary" onClick={() => setTab("import")}>
                  Import my courses…
                </button>
              </div>
            </div>

            <ul className="subject-list">
              {visibleSubjects.length === 0 && subjectFilter && (
                <li className="subject-empty muted">No subjects match &ldquo;{subjectFilter}&rdquo;.</li>
              )}
              {visibleSubjects.map((s) => (
                <li key={s.name} className={s.enabled ? "" : "off"}>
                  <div className="subject-main">
                    <div className="subject-row">
                      <strong>{s.name}</strong>
                      <label className="switch" title={s.enabled ? "Tidy sorts into this subject" : "Ignored while sorting"}>
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
                        <textarea
                          className="keywords-editor"
                          value={editKeywords}
                          onChange={(e) => setEditKeywords(e.target.value)}
                          placeholder="Phrases separated by commas — kept exactly as you write them"
                          rows={8}
                          autoFocus
                        />
                        <p className="hint">
                          {plural(splitKeywords(editKeywords).length, "phrase")} · saved exactly as
                          entered
                        </p>
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
                            className="primary-toggle start small"
                            onClick={async () => {
                              if (await saveSubject(s.name, editKeywords)) setEditing(null);
                            }}
                          >
                            Save keywords
                          </button>
                        </div>
                      </div>
                    ) : (
                      <p className="muted keywords" title={s.keywords.join(", ")}>
                        {s.keywords.length
                          ? `${plural(s.keywords.length, "keyword")} · ${s.keywords.slice(0, 6).join(" · ")}${
                              s.keywords.length > 6 ? "…" : ""
                            }`
                          : "No keywords yet — only files containing the subject name will match"}
                      </p>
                    )}
                    {editing !== s.name && (
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
                          className="text-btn danger"
                          onClick={() => removeSubject(s.name)}
                        >
                          Remove
                        </button>
                      </div>
                    )}
                  </div>
                </li>
              ))}
              <li className="subject-general">
                <div>
                  <strong>General</strong>
                  <p className="muted keywords">Catch-all for files without a clear subject.</p>
                </div>
                <span className="badge">Always on</span>
              </li>
            </ul>

            <form
              className="add-subject"
              onSubmit={async (e) => {
                e.preventDefault();
                if (!draftName.trim()) return;
                if (await saveSubject(draftName.trim(), draftKeywords)) {
                  setDraftName("");
                  setDraftKeywords("");
                }
              }}
            >
              <h3>Add a subject</h3>
              <input
                value={draftName}
                onChange={(e) => setDraftName(e.target.value)}
                placeholder="Subject name, e.g. Calculus"
                aria-label="New subject name"
              />
              {draftExists && (
                <p className="hint inline-warn">
                  A subject with this name exists — saving will replace its keywords.
                </p>
              )}
              <textarea
                className="keywords-editor"
                value={draftKeywords}
                onChange={(e) => setDraftKeywords(e.target.value)}
                placeholder="Keywords separated by commas, e.g. limits, derivatives, integration by parts"
                aria-label="New subject keywords"
                rows={4}
              />
              <div>
                <button type="submit" className="secondary" disabled={!draftName.trim()}>
                  {draftExists ? "Update subject" : "Add subject"}
                </button>
              </div>
            </form>

            <h2>File type folders</h2>
            <p className="muted intro">
              Inside each subject, files are grouped by type based on their extension.
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
          </section>
        )}

        {tab === "settings" && config && (
          <section className="settings">
            <h2>Folders</h2>
            <div className="setting-row">
              <div>
                <strong>Watch folder</strong>
                <p className="muted">Where Tidy looks for new files.</p>
                <p className="muted mono" title={config.watchFolder ?? undefined}>
                  {config.watchFolder ?? "Not set"}
                </p>
              </div>
              <div className="btn-row">
                <button type="button" className="secondary" onClick={() => pickFolder("watchFolder")}>
                  Change…
                </button>
                <button
                  type="button"
                  className="secondary"
                  disabled={!config.watchFolder}
                  onClick={() => config.watchFolder && revealPath(config.watchFolder)}
                >
                  Show
                </button>
              </div>
            </div>
            <div className="setting-row">
              <div>
                <strong>Destination folder</strong>
                <p className="muted">Where sorted files are filed, as Subject / Type / file.</p>
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
                  Change…
                </button>
                <button
                  type="button"
                  className="secondary"
                  disabled={!config.destinationFolder}
                  onClick={() => config.destinationFolder && revealPath(config.destinationFolder)}
                >
                  Show
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
              <span>
                <strong>Start watching when Tidy opens</strong>
                <small>
                  Closing the window keeps Tidy in the menu bar / system tray so it keeps
                  watching. Use <em>Quit Tidy</em> from the tray menu to fully exit.
                </small>
              </span>
            </label>

            <h2>Setup</h2>
            <div className="setting-row">
              <div>
                <strong>Run the setup guide again</strong>
                <p className="muted">
                  Walk through folders and subjects again. Your history and sorted files are kept.
                </p>
              </div>
              <button type="button" className="secondary" onClick={rerunSetup}>
                Open setup
              </button>
            </div>

            <div className="about">
              <p>
                <strong>Tidy</strong>
                {version ? ` ${version}` : ""} · runs entirely on this computer. Never
                overwrites a file (adds “(2)” instead), never deletes anything, and skips
                downloads that are still in progress.
              </p>
            </div>
          </section>
        )}
      </main>

      {config && !config.onboardingComplete && (
        <Onboarding
          config={config}
          rerun={rerun}
          busy={busy}
          error={error}
          onPickFolder={pickFolder}
          onAddSubjects={addOnboardingSubjects}
          onFinish={finishOnboarding}
        />
      )}

      {notice && (
        <div className="toast" role="status" aria-live="polite">
          <span className="toast-dot" aria-hidden />
          {notice}
        </div>
      )}

      {progress && (
        <div className="progress-overlay" role="status" aria-live="polite">
          <div className="progress-card">
            <div className="progress-head">
              <div className="progress-spinner" aria-hidden />
              <h2>
                {progress.phase === "moving"
                  ? "Moving files"
                  : progress.phase === "reading"
                    ? "Reading documents"
                    : "Matching files"}
              </h2>
              <span className="progress-count">
                {progress.total > 0 ? `${progress.current} / ${progress.total}` : "Starting…"}
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
            <p className="hint">
              {progress.phase === "moving"
                ? "Moving files into place. Existing files are never overwritten."
                : progress.phase === "reading"
                  ? "Pulling text from PDFs/DOCX when the filename alone is unclear."
                  : "Matching filenames to your subjects…"}
            </p>
          </div>
        </div>
      )}

      {preview && !progress && (
        <div
          className="modal-backdrop"
          role="presentation"
          onClick={previewKind === "pending" ? undefined : dismissPreview}
        >
          <div
            className="modal preview-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="preview-title"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-head">
              <h2 id="preview-title">
                {previewKind === "pending"
                  ? `${plural(preview.length, "new download")} ready to file`
                  : previewKind === "reorganize"
                    ? "Fix mis-sorted files"
                    : `Tidy up ${watchName}`}
              </h2>
              <button type="button" className="text-btn" onClick={dismissPreview} title="Esc">
                Close
              </button>
            </div>
            {preview.length === 0 ? (
              <>
                <p className="muted">
                  {previewKind === "reorganize"
                    ? "Everything in the destination already matches your current subjects."
                    : `There are no files in ${watchName} to sort right now.`}
                </p>
                <div className="modal-foot">
                  <button type="button" className="secondary" onClick={dismissPreview}>
                    OK
                  </button>
                </div>
              </>
            ) : (
              <>
                <p className="muted intro">
                  {previewKind === "reorganize"
                    ? "These files look like they belong in a different subject. Untick any you want to leave alone."
                    : "Here's where each file will go. Untick anything you want to leave where it is — nothing moves until you confirm."}
                  {previewStats && previewStats.unsure > 0 && (
                    <>
                      {" "}
                      <span className="unsure-note">
                        {plural(previewStats.unsure, "file")} had no clear subject and will go to
                        General.
                      </span>
                    </>
                  )}
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
                      <span className="preview-count">
                        {selected.size} of {preview.length} selected
                      </span>
                    </div>
                    <ul className="preview-list">
                      {preview.map((item) => (
                        <li
                          key={item.source}
                          className={item.subject === "General" ? "unsure" : ""}
                        >
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
                              <strong title={item.filename}>{item.filename}</strong>
                              <span className="route-tag">
                                {item.fromSubject != null && (
                                  <>
                                    {item.fromSubject}
                                    {item.fromFileType ? `/${item.fromFileType}` : ""}
                                    <i> → </i>
                                  </>
                                )}
                                {item.subject}/{item.fileType}/
                              </span>
                            </span>
                          </label>
                        </li>
                      ))}
                    </ul>
                  </div>
                  <div className="preview-tree-pane">
                    <p className="eyebrow">
                      Result
                      {previewStats && previewStats.subjects > 0
                        ? ` · ${plural(previewStats.subjects, "subject folder")}`
                        : ""}
                    </p>
                    <PreviewTree
                      rootName={destName}
                      items={preview.filter((p) => selected.has(p.source))}
                    />
                  </div>
                </div>
                <div className="modal-foot">
                  <span className="modal-foot-note">You can undo any move afterwards.</span>
                  <button type="button" className="secondary" onClick={dismissPreview}>
                    {previewKind === "pending" ? "Leave them for now" : "Cancel"}
                  </button>
                  <button
                    type="button"
                    className="primary-toggle start"
                    disabled={busy || selected.size === 0}
                    onClick={applyPreview}
                  >
                    Move {plural(selected.size, "file")}
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
