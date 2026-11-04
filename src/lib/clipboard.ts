/** Text onto the clipboard; the older way where the newer one is refused. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const field = document.createElement("textarea");
    field.value = text;
    field.setAttribute("readonly", "");
    field.style.position = "fixed";
    field.style.opacity = "0";
    // The field takes the focus to be copied from; it goes back to where it was after (a Drive
    // row, for one, whose arrows go on from it).
    const was = document.activeElement as HTMLElement | null;
    document.body.appendChild(field);
    field.select();
    let copied = false;
    try {
      copied = document.execCommand("copy");
    } catch {
      copied = false;
    }
    field.remove();
    was?.focus?.({ preventScroll: true });
    return copied;
  }
}

/**
 * The name Finder gives a copy put into a folder that already holds `taken`: its own name where
 * that is free, else "Report copy.pdf", then "Report copy 2.pdf". A copy of a copy counts on
 * ("Report copy 2.pdf"), never "Report copy copy.pdf". Compared without regard to case, as iCloud
 * Drive compares names. The same rule as the backend's `copy_name`, which names the real copy;
 * this one names the row shown while it is being made.
 */
export function copyName(stem: string, extension: string | null, taken: Iterable<string>): string {
  const ext = extension ? `.${extension}` : "";
  const used = new Set([...taken].map((t) => t.toLocaleLowerCase()));
  const free = (name: string) => !used.has(name.toLocaleLowerCase());
  if (free(`${stem}${ext}`)) return `${stem}${ext}`;
  const base = stem.match(/^(.+) copy(?: \d+)?$/)?.[1] ?? stem;
  if (free(`${base} copy${ext}`)) return `${base} copy${ext}`;
  for (let n = 2; ; n++) if (free(`${base} copy ${n}${ext}`)) return `${base} copy ${n}${ext}`;
}

/**
 * The name Finder's Keep Both gives an item put into a folder where its name is taken:
 * "Report 2.pdf", then "Report 3.pdf". Compared without regard to case, as `copyName`.
 */
export function keepBothName(stem: string, extension: string | null, taken: Iterable<string>): string {
  const ext = extension ? `.${extension}` : "";
  const used = new Set([...taken].map((t) => t.toLocaleLowerCase()));
  for (let n = 2; ; n++) if (!used.has(`${stem} ${n}${ext}`.toLocaleLowerCase())) return `${stem} ${n}${ext}`;
}
