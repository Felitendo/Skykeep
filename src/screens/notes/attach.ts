import { useEffect, useState } from "react";
import { notesLockApi } from "../../lib/ipc";

/**
 * Whether a picture or file can be added to a note — the paperclip and dropping a file on the
 * note, both. It is the backend's switch and nothing else: `NOTE_ATTACH_WRITES` in
 * crates/quince-icloud/src/notes_attach.rs, reported by `note_attach_enabled`. While that is
 * false (or the command is missing) neither control exists, so turning the backend on is all
 * it takes to bring them in.
 */
let known: boolean | null = null;
let asking: Promise<boolean> | null = null;

export function useAttachEnabled(): boolean {
  const [on, setOn] = useState(known ?? false);
  useEffect(() => {
    if (known !== null) return setOn(known);
    let live = true;
    asking ??= notesLockApi.noteAttachEnabled().then((v) => (known = v));
    void asking.then((v) => live && setOn(v));
    return () => {
      live = false;
    };
  }, []);
  return on;
}

/** A path's last part, for saying what was added. */
export const fileName = (path: string) => path.split(/[\\/]/).pop() || path;
