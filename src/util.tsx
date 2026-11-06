export function shortPath(path: string | null | undefined, max = 48): string {
  if (!path) return "Not set";
  if (path.length <= max) return path;
  return "…" + path.slice(-(max - 1));
}

export function folderName(path: string | null | undefined): string {
  if (!path) return "";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function formatTime(iso: string): string {
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

export function errText(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  return raw.replace(/^Error:\s*/, "");
}

export function plural(n: number, word: string, many = `${word}s`): string {
  return `${n} ${n === 1 ? word : many}`;
}

export function UndoIcon() {
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

export function FolderIcon() {
  return (
    <svg className="ico" viewBox="0 0 16 16" aria-hidden>
      <path
        d="M1.75 4.25c0-.69.56-1.25 1.25-1.25h3.1l1.4 1.5H13c.69 0 1.25.56 1.25 1.25v6.5c0 .69-.56 1.25-1.25 1.25H3c-.69 0-1.25-.56-1.25-1.25v-8Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </svg>
  );
}
