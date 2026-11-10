/**
 * Add Link, as Notes has it on a Mac (Edit › Add Link, Command-K): an address put on the text
 * selected, or on new text when nothing is, or changed or taken off where the cursor stands in
 * a link. The link is the editor's `link` mark, which a save writes into the note as Apple
 * keeps a link (ckdocument.rs, the run's field 9): the same link on a phone and a Mac.
 */
import type { Command, EditorState } from "prosemirror-state";
import { TextSelection } from "prosemirror-state";
import { outsideHref } from "../../lib/links";
import { schema } from "./rich";

/**
 * The address a person typed, as a link: one Skykeep opens (lib/links), or null. A scheme
 * given is kept; an email address becomes `mailto:`; anything else with a dot in it is a web
 * address, as Notes and every browser's address bar take "example.com".
 */
export function linkAddress(typed: string): string | null {
  const t = typed.trim();
  if (!t || /\s/.test(t)) return null;
  if (/^[a-z][a-z0-9+.-]*:/i.test(t) && !/^[^:/]+\.[^:/]+:\d/.test(t)) return outsideHref(t);
  if (/^[^@/]+@[^@/]+\.[^@/]+$/.test(t)) return outsideHref(`mailto:${t}`);
  if (/^[^/?#]+\.[^/?#]{2,}/.test(t) || /^localhost(:\d+)?(\/|$)/i.test(t)) return outsideHref(`https://${t}`);
  return null;
}

/** The link the cursor or selection stands in: all of its text, and where it points. */
export function linkAt(state: EditorState): { from: number; to: number; href: string } | null {
  const type = schema.marks.link;
  const { $from, from, to } = state.selection;
  const mark = type.isInSet($from.marks()) ?? type.isInSet($from.nodeAfter?.marks ?? []);
  if (!mark) return null;
  // The whole run of text carrying this same link, in the line the selection starts in.
  const start = $from.start();
  let found: { from: number; to: number } | null = null;
  let run: { from: number; to: number } | null = null;
  $from.parent.forEach((child, offset) => {
    const a = start + offset;
    const b = a + child.nodeSize;
    if (mark.isInSet(child.marks)) {
      run = run && run.to === a ? { from: run.from, to: b } : { from: a, to: b };
      if (run.from <= from && from <= run.to) found = { ...run };
    } else {
      run = null;
    }
  });
  const range = found as { from: number; to: number } | null;
  if (!range || to > range.to) return null;
  return { ...range, href: mark.attrs.href as string };
}

/**
 * The link set: on the selection; on the link the cursor stands in; or, with nothing selected
 * and no link there, as `text` (the address itself when no text is given) put in at the
 * cursor. The cursor ends after the link, so typing goes on as plain text. `href` null takes
 * the link off, leaving its text.
 */
export function setLink(href: string | null, text?: string): Command {
  return (state, dispatch) => {
    const type = schema.marks.link;
    const { empty, from, to } = state.selection;
    const around = linkAt(state);
    const range = !empty ? { from, to } : around;
    if (href === null) {
      if (!range) return false;
      dispatch?.(state.tr.removeMark(range.from, range.to, type).scrollIntoView());
      return true;
    }
    const mark = type.create({ href });
    const tr = state.tr;
    let end: number;
    if (range) {
      tr.removeMark(range.from, range.to, type).addMark(range.from, range.to, mark);
      end = range.to;
    } else {
      const words = (text ?? "").trim() || href.replace(/^mailto:|^tel:/, "");
      const marks = state.storedMarks ?? state.selection.$from.marks();
      tr.insert(from, schema.text(words, mark.addToSet(marks.filter((m) => m.type !== type))));
      end = from + words.length;
    }
    tr.setSelection(TextSelection.create(tr.doc, end)).removeStoredMark(type);
    dispatch?.(tr.scrollIntoView());
    return true;
  };
}
