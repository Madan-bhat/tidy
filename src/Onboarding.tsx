import { useState } from "react";
import type { AppConfig } from "./types";
import { FolderIcon, folderName, plural, shortPath } from "./util";

const STEPS = ["Welcome", "Folders", "Subjects", "Finish"] as const;

type SubjectChoice = "type" | "import" | "later" | "keep";

const COURSES_PLACEHOLDER = `One course per line. Topics after a colon help matching:

Calculus: limits, derivatives, integrals
Organic Chemistry: alkanes, reactions
World History`;

export interface FinishOptions {
  startMonitoring: boolean;
  monitorOnLaunch: boolean;
  organizeExisting: boolean;
  openImport: boolean;
}

interface OnboardingProps {
  config: AppConfig;
  /** Re-opened from Settings: offer to keep the subjects already configured. */
  rerun: boolean;
  busy: boolean;
  error: string | null;
  onPickFolder: (field: "watchFolder" | "destinationFolder") => void;
  /** Parses a course list and saves the resulting subjects. */
  onAddSubjects: (text: string) => Promise<boolean>;
  onFinish: (opts: FinishOptions) => void;
}

export default function Onboarding({
  config,
  rerun,
  busy,
  error,
  onPickFolder,
  onAddSubjects,
  onFinish,
}: OnboardingProps) {
  const hasSubjects = config.subjects.length > 0;
  const [step, setStep] = useState(0);
  const [choice, setChoice] = useState<SubjectChoice>(rerun && hasSubjects ? "keep" : "type");
  const [courses, setCourses] = useState("");
  const [savedCourses, setSavedCourses] = useState<string | null>(null);
  const [startNow, setStartNow] = useState(true);
  const [onLaunch, setOnLaunch] = useState(config.monitorOnLaunch);
  const [organizeExisting, setOrganizeExisting] = useState(false);

  const foldersReady = Boolean(config.watchFolder && config.destinationFolder);
  const sameFolder = foldersReady && config.watchFolder === config.destinationFolder;
  const destName = folderName(config.destinationFolder) || "Destination";
  const watchName = folderName(config.watchFolder) || "the watch folder";
  const exampleSubject = config.subjects.find((s) => s.enabled)?.name ?? "Calculus";

  async function continueFromSubjects() {
    if (choice === "type" && courses.trim() && savedCourses !== courses) {
      const ok = await onAddSubjects(courses);
      if (!ok) return;
      setSavedCourses(courses);
    }
    setStep(3);
  }

  function finish() {
    onFinish({
      startMonitoring: startNow,
      monitorOnLaunch: onLaunch,
      organizeExisting,
      openImport: choice === "import",
    });
  }

  const subjectsSummary =
    choice === "import"
      ? "Import right after setup"
      : hasSubjects
        ? plural(config.subjects.length, "subject")
        : "None yet — files go to General";

  return (
    <div className="onboarding" role="dialog" aria-modal="true" aria-labelledby="ob-title">
      <div className="ob-sheet">
        <div className="ob-top">
          <ol className="ob-progress" aria-label="Setup progress">
            {STEPS.map((label, i) => (
              <li
                key={label}
                className={i < step ? "done" : i === step ? "current" : ""}
                aria-current={i === step ? "step" : undefined}
              >
                <span className="ob-progress-bar" aria-hidden />
                <span className="ob-progress-label">
                  {i + 1}. {label}
                </span>
              </li>
            ))}
          </ol>
          {step < 3 && (
            <button
              type="button"
              className="text-btn ob-skip"
              disabled={busy}
              title="You can run setup again any time from Settings"
              onClick={() =>
                onFinish({
                  startMonitoring: false,
                  monitorOnLaunch: config.monitorOnLaunch,
                  organizeExisting: false,
                  openImport: false,
                })
              }
            >
              Skip for now
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
                Tidy keeps your Downloads folder clean by filing each new file into a{" "}
                <strong>Subject / Type</strong> folder that you define — like{" "}
                <span className="mono ob-inline">Calculus/Documents/</span>. Setup takes about a
                minute.
              </p>

              <ol className="ob-how" aria-label="How Tidy works">
                <li>
                  <span className="ob-how-n">1</span>
                  <strong>Watch</strong>
                  <p>Notices new files in Downloads and waits until each download has fully finished.</p>
                </li>
                <li>
                  <span className="ob-how-n">2</span>
                  <strong>Match</strong>
                  <p>Reads the filename — and the text inside PDFs, Word and text files — to pick one of your subjects.</p>
                </li>
                <li>
                  <span className="ob-how-n">3</span>
                  <strong>You confirm</strong>
                  <p>Shows exactly where every file will go. Nothing moves until you say so.</p>
                </li>
              </ol>

              <div className="ob-example" aria-label="Example">
                <span className="mono">limits_worksheet_3.pdf</span>
                <span className="ob-example-wire" aria-hidden />
                <span className="mono">
                  <b>Calculus</b>/Documents/
                </span>
              </div>

              <p className="ob-trust">
                Runs on this computer · No account · Your files are never uploaded · Every move
                can be undone
              </p>
            </>
          )}

          {step === 1 && (
            <>
              <span className="eyebrow">Step 2 of 4 · Folders</span>
              <h2 id="ob-title">Pick where files come from, and where they should go.</h2>
              <div className="ob-folders">
                <div className="ob-folder">
                  <div>
                    <strong>
                      <FolderIcon /> Watch folder
                    </strong>
                    <p className="muted">
                      Tidy looks for new files here — usually Downloads. We&rsquo;ve pre-selected it
                      if we could find it.
                    </p>
                    <p
                      className={`mono ob-path ${config.watchFolder ? "" : "unset"}`}
                      title={config.watchFolder ?? undefined}
                    >
                      {shortPath(config.watchFolder, 56)}
                    </p>
                  </div>
                  <button
                    type="button"
                    className="secondary"
                    disabled={busy}
                    onClick={() => onPickFolder("watchFolder")}
                  >
                    {config.watchFolder ? "Change…" : "Choose…"}
                  </button>
                </div>
                <span className="ob-folder-arrow" aria-hidden />
                <div className={`ob-folder ${config.destinationFolder ? "" : "needs"}`}>
                  <div>
                    <strong>
                      <FolderIcon /> Destination folder
                    </strong>
                    <p className="muted">
                      Sorted files are filed into subfolders here. A new, empty folder such as{" "}
                      <span className="mono">Documents/Tidy</span> works well — the picker has a
                      &ldquo;New Folder&rdquo; button.
                    </p>
                    <p
                      className={`mono ob-path ${config.destinationFolder ? "" : "unset"}`}
                      title={config.destinationFolder ?? undefined}
                    >
                      {config.destinationFolder
                        ? shortPath(config.destinationFolder, 56)
                        : "Not chosen yet"}
                    </p>
                  </div>
                  <button
                    type="button"
                    className={config.destinationFolder ? "secondary" : "primary-toggle start ob-choose"}
                    disabled={busy}
                    onClick={() => onPickFolder("destinationFolder")}
                  >
                    {config.destinationFolder ? "Change…" : "Choose…"}
                  </button>
                </div>
              </div>

              {sameFolder ? (
                <p className="ob-warn" role="status">
                  These are the same folder. Pick a separate destination so sorted files
                  aren&rsquo;t picked up again.
                </p>
              ) : (
                <p className="ob-preview-path" aria-live="polite">
                  <span className="eyebrow">A sorted file will end up at</span>
                  <span className="mono">
                    {destName}/<b>{exampleSubject}</b>/<b>Documents</b>/notes.pdf
                  </span>
                </p>
              )}
            </>
          )}

          {step === 2 && (
            <>
              <span className="eyebrow">Step 3 of 4 · Subjects</span>
              <h2 id="ob-title">What are you studying?</h2>
              <p className="ob-sub">
                Each subject becomes a folder. Tidy matches files to it by its name and the
                topics you list. You can edit everything later in <em>Rules</em>.
              </p>
              <div className="ob-choices" role="radiogroup" aria-label="Subjects">
                {rerun && hasSubjects && (
                  <label className={`ob-choice ${choice === "keep" ? "on" : ""}`}>
                    <input
                      type="radio"
                      name="ob-subjects"
                      checked={choice === "keep"}
                      onChange={() => setChoice("keep")}
                    />
                    <span>
                      <strong>Keep my {plural(config.subjects.length, "subject")}</strong>
                      <span className="ob-chips" aria-hidden>
                        {config.subjects.slice(0, 5).map((s) => (
                          <i key={s.name}>{s.name}</i>
                        ))}
                        {config.subjects.length > 5 && (
                          <i className="more">+{config.subjects.length - 5} more</i>
                        )}
                      </span>
                    </span>
                  </label>
                )}
                <label className={`ob-choice ${choice === "type" ? "on" : ""}`}>
                  <input
                    type="radio"
                    name="ob-subjects"
                    checked={choice === "type"}
                    onChange={() => setChoice("type")}
                  />
                  <span>
                    <strong>Type my courses here</strong>
                    <small>Quickest — a short list is enough to get started.</small>
                  </span>
                </label>
                {choice === "type" && (
                  <textarea
                    className="ob-courses"
                    value={courses}
                    onChange={(e) => setCourses(e.target.value)}
                    placeholder={COURSES_PLACEHOLDER}
                    rows={5}
                    spellCheck={false}
                    autoFocus
                    aria-label="Your courses, one per line"
                  />
                )}
                <label className={`ob-choice ${choice === "import" ? "on" : ""}`}>
                  <input
                    type="radio"
                    name="ob-subjects"
                    checked={choice === "import"}
                    onChange={() => setChoice("import")}
                  />
                  <span>
                    <strong>Import a full syllabus after setup</strong>
                    <small>Paste a long syllabus or load a text file, then review each subject.</small>
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
                    <strong>I&rsquo;ll add them later</strong>
                    <small>Until you do, files are still sorted by type inside a General folder.</small>
                  </span>
                </label>
              </div>
              <details className="ob-explain">
                <summary>How does Tidy decide the subject?</summary>
                <p>
                  A file called <span className="mono">limits_worksheet_3.pdf</span> goes to{" "}
                  <strong>Calculus</strong> because &ldquo;limits&rdquo; is one of its topics. If
                  the filename isn&rsquo;t enough, Tidy reads the text inside PDF, DOCX and TXT
                  files. When no subject is a clear winner the file goes to <strong>General</strong>{" "}
                  instead of being guessed — and you review every move before it happens.
                </p>
              </details>
            </>
          )}

          {step === 3 && (
            <>
              <span className="eyebrow">Step 4 of 4 · Finish</span>
              <h2 id="ob-title">You&rsquo;re all set.</h2>
              <div className="ob-summary">
                <div>
                  <span className="eyebrow">Watching</span>
                  <p className="mono" title={config.watchFolder ?? undefined}>
                    {shortPath(config.watchFolder, 34)}
                  </p>
                </div>
                <div>
                  <span className="eyebrow">Filing into</span>
                  <p className="mono" title={config.destinationFolder ?? undefined}>
                    {shortPath(config.destinationFolder, 34)}
                  </p>
                </div>
                <div>
                  <span className="eyebrow">Subjects</span>
                  <p>{subjectsSummary}</p>
                </div>
              </div>

              <div className="ob-options">
                <label className="setting-check">
                  <input
                    type="checkbox"
                    checked={startNow}
                    onChange={(e) => setStartNow(e.target.checked)}
                  />
                  <span>
                    <strong>Start watching now</strong>
                    <small>New downloads will be proposed for review as they arrive.</small>
                  </span>
                </label>
                <label className="setting-check">
                  <input
                    type="checkbox"
                    checked={onLaunch}
                    onChange={(e) => setOnLaunch(e.target.checked)}
                  />
                  <span>
                    <strong>Start watching whenever Tidy opens</strong>
                    <small>Tidy stays in the menu bar / system tray when you close the window.</small>
                  </span>
                </label>
                <label className="setting-check">
                  <input
                    type="checkbox"
                    checked={organizeExisting}
                    onChange={(e) => setOrganizeExisting(e.target.checked)}
                  />
                  <span>
                    <strong>Also tidy the files already in {watchName}</strong>
                    <small>You&rsquo;ll see a preview first and can untick anything.</small>
                  </span>
                </label>
              </div>

              <p className="muted ob-after">
                Changed your mind about a move? Use <em>Undo</em> on any file, or{" "}
                <em>Revert last batch</em> on Home.
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
          {step > 0 ? (
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
              {!foldersReady && <span className="muted">Choose a destination to continue</span>}
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
            <div className="ob-next">
              {choice === "type" && !courses.trim() && (
                <span className="muted">Type at least one course</span>
              )}
              <button
                type="button"
                className="primary-toggle start"
                disabled={busy || (choice === "type" && !courses.trim())}
                onClick={continueFromSubjects}
              >
                {busy ? "Saving…" : choice === "type" ? "Save subjects & continue" : "Continue"}
              </button>
            </div>
          )}
          {step === 3 && (
            <button type="button" className="primary-toggle start" disabled={busy} onClick={finish}>
              {choice === "import" ? "Finish & import syllabus" : "Finish setup"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
