/**
 * Web addresses in a note are links the moment they are there, as Notes' own data detectors
 * make them: typed, pasted, or written on another device without a link of its own. Drawn as
 * links (decorations), never written into the note — the note keeps exactly what Apple has.
 */
import { Plugin } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import type { Node as PMNode } from "prosemirror-model";

/** An address: with its scheme, or starting www. — ending before closing punctuation, as a
 * sentence's full stop or a bracket is not part of it. */
const ADDRESS = /\b(?:https?:\/\/|www\.)[^\s<>"'`]*[^\s<>"'`.,;:!?)\]}]/gi;

export type Found = { from: number; to: number; href: string };

/** The addresses in a piece of text, with where each starts and ends, and what it opens. */
export function addressesIn(text: string): Found[] {
  const out: Found[] = [];
  for (const m of text.matchAll(ADDRESS)) {
    const from = m.index ?? 0;
    const href = /^www\./i.test(m[0]) ? `https://${m[0]}` : m[0];
    out.push({ from, to: from + m[0].length, href });
  }
  return out;
}

function decorate(doc: PMNode): DecorationSet {
  const found: Decoration[] = [];
  doc.descendants((node, pos) => {
    if (!node.isText || !node.text) return;
    // A link of its own (made on the phone, or here) is drawn as that already.
    if (node.marks.some((m) => m.type.name === "link")) return;
    for (const a of addressesIn(node.text)) {
      found.push(Decoration.inline(pos + a.from, pos + a.to, { nodeName: "a", href: a.href, class: "nf-autolink" }));
    }
  });
  return DecorationSet.create(doc, found);
}

/** Addresses found again only when the text changed; a selection moving keeps them. */
export const autolink = new Plugin<DecorationSet>({
  state: {
    init: (_, state) => decorate(state.doc),
    apply: (tr, was) => (tr.docChanged ? decorate(tr.doc) : was),
  },
  props: {
    decorations(state) {
      return this.getState(state);
    },
  },
});

/** A click on a link (its own, or one found) opens it, as a click does in Notes on the Mac.
 * Navigating is enough: the app hands any address outside it to this computer's browser. */
/** A link to another note asked to be followed: the Notes screen (NotesView) opens it. */
export const OPEN_NOTE_EVENT = "quince:open-note";

/** The note an Apple Notes link points at, as the library names it: `applenotes:note/<id>?…`
 * carries the note's record name, lower-cased (measured on his "Link feature of notes"). */
export function linkedNote(href: string): string | null {
  const m = /^applenotes:(?:\/\/)?note\/([0-9a-f-]{36})/i.exec(href);
  return m ? m[1].toUpperCase() : null;
}

export function openLinkAt(event: MouseEvent): boolean {
  const a = (event.target as HTMLElement | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
  if (!a || event.button !== 0 || event.shiftKey || event.altKey) return false;
  const href = a.getAttribute("href") ?? "";
  // A link to another note opens that note here, as it does in Notes.
  const note = linkedNote(href);
  if (note) {
    event.preventDefault();
    window.dispatchEvent(new CustomEvent<string>(OPEN_NOTE_EVENT, { detail: note }));
    return true;
  }
  if (!/^(https?:|mailto:|tel:)/i.test(href)) return false;
  event.preventDefault();
  window.location.assign(href);
  return true;
}
