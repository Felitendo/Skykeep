/**
 * A note's picture shown large, as clicking one in Notes on the Mac does: over the dimmed
 * window, at once — the picture already drawn in the note, nothing fetched again — with its
 * name and a way to open it in another app. Escape, the close button or a click outside put
 * it away.
 */
import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { Portal } from "../../components/Portal";
import { api, messageOf } from "../../lib/ipc";
import { PICTURE_EVENT, type PictureShown } from "./NoteEditor";

export function NotePicture() {
  const [shown, setShown] = useState<PictureShown | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    const show = (e: Event) => {
      setProblem(null);
      setShown((e as CustomEvent<PictureShown>).detail);
    };
    window.addEventListener(PICTURE_EVENT, show);
    return () => window.removeEventListener(PICTURE_EVENT, show);
  }, []);
  useEffect(() => {
    if (!shown) return undefined;
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setShown(null);
      }
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [shown]);
  if (!shown) return null;
  return (
    <Portal>
      <div className="modal-veil nf-picture-veil" onClick={() => setShown(null)}>
        <figure
          className="nf-picture"
          role="dialog"
          aria-modal="true"
          aria-label={shown.name}
          onClick={(e) => e.stopPropagation()}
        >
          <img src={shown.src} alt={shown.name} draggable={false} />
          <figcaption>
            <span className="nf-picture-name">{problem ?? shown.name}</span>
            <button
              className="quiet"
              onClick={() => void api.noteAttachmentOpen(shown.id).catch((e) => setProblem(messageOf(e)))}
            >
              Open in Default App
            </button>
            <button className="nf-picture-close" aria-label="Close" autoFocus onClick={() => setShown(null)}>
              <X size={16} strokeWidth={2} />
            </button>
          </figcaption>
        </figure>
      </div>
    </Portal>
  );
}
