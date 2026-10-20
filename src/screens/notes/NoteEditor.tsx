import "prosemirror-view/style/prosemirror.css";
import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { EditorState, TextSelection, type Command, type Transaction } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { history, undo, redo } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import { baseKeymap, splitBlockAs, toggleMark, chainCommands } from "prosemirror-commands";
import { Fragment, Slice, type Node as PMNode } from "prosemirror-model";
import type { Run } from "./Formatted";
import { fromDoc, LIST_STYLES, schema, STYLE, tableCells, toDoc } from "./rich";
import { api, noteMediaSrc, type NoteAttachment } from "../../lib/ipc";
import { RowMenu, type MenuEntry } from "../../components/RowMenu";
import { autolink, openLinkAt } from "./links";

/** `content` with every attachment taken out, a table's cells kept as text in its place. */
function withoutAttachments(content: Fragment): Fragment {
  const kept: PMNode[] = [];
  content.forEach((node) => {
    if (node.type === schema.nodes.attachment) {
      const text = node.textContent || (node.type.spec.leafText?.(node) ?? "");
      if (text) kept.push(schema.text(text.replace(/\s*\n\s*/g, " ")));
      return;
    }
    kept.push(node.isLeaf ? node : node.copy(withoutAttachments(node.content)));
  });
  return Fragment.fromArray(kept);
}

/** Every table on screen, by its element: how to read it again. */
const liveTables = new Map<HTMLElement, () => void>();

/** Every table on screen read again — after a sync, for a change made on another device. */
export function refreshTables() {
  liveTables.forEach((read) => read());
}

/** Whether tables can be edited here. Off until an edit made here is seen on an iPhone. */
const TABLE_WRITES = false;

/** A table's right-click menu: drawn by the editor that holds the table, opened by a cell. */
type CellMenu = { at: { x: number; y: number; alignRight: boolean }; entries: MenuEntry[] };
let openCellMenu: ((menu: CellMenu) => void) | null = null;

/**
 * An attachment in the note: a table drawn as the grid it is, with the cells Apple holds;
 * anything else — a picture, a sketch — as a labelled placeholder that keeps its place.
 *
 * A table's cells are edited where they stand. Each cell is a plain-text field of its own —
 * keys typed there are the cell's, never the note's — saved to Apple when it is left or
 * Return is pressed, drawn at once, and put back with the reason if Apple refuses. An empty
 * cell is editable too: a table made on a phone or on icloud.com already holds an empty text
 * object for every cell (measured on every table in his account).
 */
/** What each attachment is, asked once a session: a note re-drawn does not ask again. */
const described = new Map<string, Promise<NoteAttachment | null>>();
function describe(id: string): Promise<NoteAttachment | null> {
  let known = described.get(id);
  if (!known) {
    known = api.noteAttachmentInfo(id).catch(() => null);
    described.set(id, known);
  }
  return known;
}

function size(bytes: number | null): string {
  if (!bytes) return "";
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/**
 * A picture, document, recording or file where it sits in its note, as Notes shows it: a
 * picture at its own proportions, a PDF, scan or film as Apple's rendering of it with its
 * name, anything else as a labelled chip. Clicking one opens it in the app this computer uses
 * for it. A picture that cannot be had falls back to the chip, never a broken frame.
 */
/** A note's picture asked to be shown large: the viewer (NotePicture) listens for it. */
export const PICTURE_EVENT = "quince:note-picture";
export type PictureShown = { id: string; name: string; src: string };

function drawMedia(dom: HTMLElement, info: NoteAttachment) {
  const name = info.filename ?? info.title ?? (info.kind === "audio" ? "Recording" : "Attachment");
  const open = () => void api.noteAttachmentOpen(info.id).catch(() => {});
  const chip = () => {
    const c = document.createElement("button");
    c.type = "button";
    c.className = `nf-file is-${info.kind}`;
    const glyph = document.createElement("span");
    glyph.className = "nf-file-glyph";
    // The kind of file, as Finder labels a document: its extension, short.
    const ext = name.includes(".") ? name.split(".").pop()!.slice(0, 4).toUpperCase() : "";
    glyph.textContent = info.kind === "audio" ? "♪" : info.kind === "video" ? "▶" : ext || "FILE";
    const label = document.createElement("span");
    label.className = "nf-file-name";
    label.textContent = name;
    const meta = document.createElement("span");
    meta.className = "nf-file-meta";
    meta.textContent = size(info.size);
    c.append(glyph, label, meta);
    c.title = `Open ${name}`;
    c.addEventListener("click", open);
    return c;
  };
  const picture = (part: "file" | "preview", cls: string) => {
    const img = document.createElement("img");
    img.className = cls;
    img.alt = name;
    img.decoding = "async";
    img.draggable = false;
    if (info.width && info.height) img.style.aspectRatio = `${info.width} / ${info.height}`;
    img.src = noteMediaSrc(info.id, part);
    img.addEventListener("error", () => dom.replaceChildren(chip()));
    return img;
  };
  dom.title = `Open ${name}`;
  if (info.kind === "image") {
    const img = picture("file", "nf-image");
    // Shown here at once, from the picture already drawn — never a wait on another app.
    img.addEventListener("click", () =>
      window.dispatchEvent(new CustomEvent<PictureShown>(PICTURE_EVENT, { detail: { id: info.id, name, src: img.src } })),
    );
    dom.className = "nf-attachment is-image";
    dom.replaceChildren(img);
  } else if ((info.kind === "document" || info.kind === "video") && info.hasPreview) {
    const figure = document.createElement("button");
    figure.type = "button";
    figure.className = `nf-doc is-${info.kind}`;
    const caption = document.createElement("span");
    caption.className = "nf-doc-name";
    caption.textContent = name;
    figure.append(picture("preview", "nf-doc-preview"), caption);
    figure.addEventListener("click", open);
    dom.className = "nf-attachment is-doc";
    dom.replaceChildren(figure);
  } else {
    dom.className = "nf-attachment is-file";
    dom.replaceChildren(chip());
  }
}

function attachmentView(node: PMNode) {
  const dom = document.createElement("span");
  dom.className = "nf-attachment";
  dom.contentEditable = "false";
  dom.textContent = "Attachment";
  dom.title = "Open it on an Apple device to change it";
  const id = node.attrs.id as string | null;

  const draw = (cells: string[][]) => {
    if (id) tableCells.set(id, cells.map((row) => [...row]));
    const table = document.createElement("table");
    table.className = "nf-table";
    cells.forEach((row, r) => {
      const tr = table.insertRow();
      row.forEach((cell, c) => {
        const td = tr.insertCell();
        const was = cell.trimEnd();
        // Read-only for now: what Quince wrote to a table was stored at Apple but did not show
        // on his iPhone, so no table is written until a write is seen there.
        if (!id || !TABLE_WRITES) {
          td.textContent = was;
          td.className = was ? "nf-cell" : "nf-cell-empty";
          return;
        }
        // Right-click, as in Notes: a row added above or below this one.
        td.addEventListener("contextmenu", (e) => {
          e.preventDefault();
          const add = (at: number, column = false) => () => {
            dom.classList.add("is-saving");
            (column ? api.noteTableAddColumn(id, at) : api.noteTableAddRow(id, at)).then(
              (next) => { dom.classList.remove("is-saving"); draw(next); },
              (err) => { dom.classList.remove("is-saving"); dom.title = String(err); },
            );
          };
          const remove = (at: number, column: boolean) => () => {
            dom.classList.add("is-saving");
            api.noteTableRemove(id, at, column).then(
              (next) => { dom.classList.remove("is-saving"); draw(next); },
              (err) => { dom.classList.remove("is-saving"); dom.title = String(err); },
            );
          };
          openCellMenu?.({
            at: { x: e.clientX, y: e.clientY, alignRight: false },
            entries: [
              { label: "Add Row Above", run: add(r) },
              { label: "Add Row Below", run: add(r + 1) },
              null,
              { label: "Add Column Before", run: add(c, true) },
              { label: "Add Column After", run: add(c + 1, true) },
              null,
              // A table keeps at least one row and one column, as Notes does.
              ...(cells.length > 1 ? [{ label: "Delete Row", danger: true, run: remove(r, false) }] : []),
              ...((cells[0]?.length ?? 0) > 1 ? [{ label: "Delete Column", danger: true, run: remove(c, true) }] : []),
            ],
          });
        });
        const field = document.createElement("div");
        field.className = "nf-cell-edit";
        field.contentEditable = "plaintext-only";
        field.spellcheck = true;
        field.textContent = was;
        let saved = was;
        const save = () => {
          const now = (field.textContent ?? "").replace(/\n+/g, " ");
          if (now === saved) return;
          const before = saved;
          saved = now;
          field.classList.add("is-saving");
          const copy = tableCells.get(id);
          if (copy?.[r]) copy[r][c] = now;
          api.noteTableSetCell(id, r, c, now).then(
            () => field.classList.remove("is-saving"),
            (e) => {
              // Refused: back as Apple has it, and why.
              saved = before;
              field.textContent = before;
              if (copy?.[r]) copy[r][c] = before;
              field.classList.remove("is-saving");
              field.title = String(e);
              field.classList.add("is-refused");
              window.setTimeout(() => { field.classList.remove("is-refused"); field.title = ""; }, 4000);
            },
          );
        };
        field.addEventListener("keydown", (e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            field.blur();
          } else if (e.key === "Escape") {
            field.textContent = saved;
            field.blur();
          }
        });
        field.addEventListener("blur", save);
        td.appendChild(field);
      });
    });
    dom.className = "nf-attachment is-table";
    dom.title = "";
    dom.replaceChildren(table);
  };

  if (id) {
    api.noteTable(id).then(
      (cells) => {
        if (cells && cells.length) draw(cells);
        else describe(id).then((info) => info && drawMedia(dom, info), () => {});
      },
      () => {},
    );
    // Read again after every sync, so a table typed in on a phone shows its new cells here
    // without leaving the note; drawn again only when they changed.
    liveTables.set(dom, () => {
      if (!dom.classList.contains("is-table")) return;
      api.noteTable(id).then(
        (cells) => {
          if (cells && cells.length && JSON.stringify(cells) !== JSON.stringify(tableCells.get(id))) draw(cells);
        },
        () => {},
      );
    });
  }
  return {
    dom,
    destroy: () => void liveTables.delete(dom),
    ignoreMutation: () => true,
    // A cell's keys, clicks and typing are the cell's, not the note editor's.
    stopEvent: (e: Event) => Boolean((e.target as HTMLElement | null)?.closest?.(".nf-cell-edit, .nf-image, .nf-doc, .nf-file")),
  };
}

/** What the toolbar shows as on: the style of the line the cursor is in, and its marks. */
export type EditorStatus = {
  style: number | null;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  checklist: boolean;
};

export type EditorHandle = {
  /** Set every line the selection touches to one of Notes' styles. */
  setStyle: (style: number | null) => void;
  /** Lines to checklist items, or — when they all are — back to plain lines. */
  toggleChecklist: () => void;
  toggleMark: (mark: "bold" | "italic" | "underline" | "strike") => void;
  /** Take in text Apple now holds, keeping the cursor where it was as near as it can. */
  replace: (text: string, runs: Run[] | null) => void;
  focus: () => void;
  /** Hand over any change not yet reported, now. */
  flushChange: () => void;
};

type Props = {
  text: string;
  runs: Run[] | null;
  editable: boolean;
  label: string;
  placeholder?: string;
  autoFocus?: boolean;
  onChange: (text: string, runs: Run[]) => void;
  onStatus?: (status: EditorStatus) => void;
  onBlur?: () => void;
};

/** Every paragraph the selection touches, with where it starts. */
function linesIn(state: EditorState): { node: PMNode; pos: number }[] {
  const out: { node: PMNode; pos: number }[] = [];
  const { from, to } = state.selection;
  state.doc.nodesBetween(from, to, (node, pos) => {
    if (node.type === schema.nodes.paragraph) out.push({ node, pos });
    return node.type === schema.nodes.doc;
  });
  return out;
}

function restyle(style: number | null, tick?: (was: PMNode) => boolean | null): Command {
  return (state, dispatch) => {
    const tr = state.tr;
    for (const { node, pos } of linesIn(state)) {
      const checked = style === STYLE.checklist ? (tick ? tick(node) : (node.attrs.style === STYLE.checklist ? node.attrs.checked : false)) : null;
      tr.setNodeMarkup(pos, undefined, {
        ...node.attrs,
        style,
        checked,
        // Out of a list, a line is not indented as one.
        indent: style !== null && LIST_STYLES.includes(style) ? node.attrs.indent : 0,
      });
    }
    dispatch?.(tr.scrollIntoView());
    return true;
  };
}

/** Indent or outdent the lines in a list, as Tab and Command-] do. */
function shift(by: 1 | -1, onlyLists: boolean): Command {
  return (state, dispatch) => {
    const lines = linesIn(state);
    if (onlyLists && !lines.every(({ node }) => LIST_STYLES.includes(node.attrs.style))) return false;
    const tr = state.tr;
    for (const { node, pos } of lines) {
      tr.setNodeMarkup(pos, undefined, { ...node.attrs, indent: Math.max(0, Math.min(8, node.attrs.indent + by)) });
    }
    dispatch?.(tr);
    return true;
  };
}

const parentOf = (state: EditorState) => state.selection.$from.parent;
const inList = (state: EditorState) => LIST_STYLES.includes(parentOf(state).attrs.style);

/**
 * Return as Notes has it: in a list the next line is another item — a new checklist item
 * unticked — and Return on an empty item ends the list; after a title or heading the next
 * line is body text.
 */
const enter: Command = (state, dispatch) => {
  const p = parentOf(state);
  if (inList(state) && p.content.size === 0 && state.selection.empty) {
    return restyle(null)(state, dispatch);
  }
  return splitBlockAs((node, atEnd) => {
    const style = node.attrs.style as number | null;
    if (LIST_STYLES.includes(style as number)) {
      // The next item counts on from this one; only a list's first item holds where it starts.
      return { type: schema.nodes.paragraph, attrs: { ...node.attrs, checked: style === STYLE.checklist ? false : null, start: null } };
    }
    if (atEnd && (style === STYLE.title || style === STYLE.heading || style === STYLE.subheading)) {
      return { type: schema.nodes.paragraph, attrs: { style: null, indent: 0, checked: null } };
    }
    return { type: schema.nodes.paragraph, attrs: { ...node.attrs } };
  })(state, dispatch);
};

/** Backspace at the very start of a list item makes it a plain line, before joining lines. */
const backspace: Command = (state, dispatch) => {
  const { $from, empty } = state.selection;
  if (!empty || $from.parentOffset !== 0 || !inList(state)) return false;
  return restyle(null)(state, dispatch);
};

const lineBreak: Command = (state, dispatch) => {
  dispatch?.(state.tr.replaceSelectionWith(schema.nodes.line_break.create()).scrollIntoView());
  return true;
};

const tab: Command = (state, dispatch) => {
  if (shift(1, true)(state, dispatch)) return true;
  dispatch?.(state.tr.insertText("\t"));
  return true;
};

function statusOf(state: EditorState): EditorStatus {
  const p = parentOf(state);
  const marks = state.storedMarks ?? state.selection.$from.marks();
  const { from, to, empty } = state.selection;
  const has = (name: string) => {
    const type = schema.marks[name];
    return empty ? marks.some((m) => m.type === type) : state.doc.rangeHasMark(from, to, type);
  };
  return {
    style: p.attrs.style,
    bold: has("bold"),
    italic: has("italic"),
    underline: has("underline"),
    strike: has("strike"),
    checklist: p.attrs.style === STYLE.checklist,
  };
}

/**
 * A note, edited formatted — as Notes edits it on icloud.com, a Mac or a phone. Typing, the
 * Aa styles, the checklist button and the keys all change the same document, which is read
 * back as Apple's text and runs on every change.
 */
export const NoteEditor = forwardRef<EditorHandle, Props>(function NoteEditor(
  { text, runs, editable, label, placeholder, autoFocus, onChange, onStatus, onBlur },
  ref,
) {
  const host = useRef<HTMLDivElement | null>(null);
  const view = useRef<EditorView | null>(null);
  const callbacks = useRef({ onChange, onStatus, onBlur });
  callbacks.current = { onChange, onStatus, onBlur };

  /**
   * The note read back as Apple's text and runs — a walk of the whole note — done just after
   * the key is drawn rather than before, so a long note types as fast as a short one. Done at
   * once whenever the note is left, so nothing typed can be missed.
   */
  const later = useRef<number | null>(null);
  const report = () => {
    if (later.current !== null) {
      window.clearTimeout(later.current);
      later.current = null;
    }
    const v = view.current;
    if (!v) return;
    const out = fromDoc(v.state.doc);
    callbacks.current.onChange(out.text, out.runs);
  };
  const reportSoon = () => {
    if (later.current !== null) return;
    later.current = window.setTimeout(() => {
      later.current = null;
      report();
    }, 0);
  };

  useEffect(() => {
    if (!host.current) return undefined;
    const state = EditorState.create({
      doc: toDoc(text, runs),
      plugins: [
        autolink,
        history(),
        keymap({
          "Mod-z": undo,
          "Mod-Shift-z": redo,
          "Mod-y": redo,
          "Mod-b": toggleMark(schema.marks.bold),
          "Mod-i": toggleMark(schema.marks.italic),
          "Mod-u": toggleMark(schema.marks.underline),
          "Mod-]": shift(1, false),
          "Mod-[": shift(-1, false),
          Tab: tab,
          "Shift-Tab": shift(-1, false),
          Enter: enter,
          "Shift-Enter": lineBreak,
          Backspace: backspace,
          // The Mac's Format ▸ keys for Notes' styles.
          "Mod-Shift-t": restyle(STYLE.title),
          "Mod-Shift-h": restyle(STYLE.heading),
          "Mod-Shift-j": restyle(STYLE.subheading),
          "Mod-Shift-b": restyle(null),
          "Mod-Shift-m": restyle(STYLE.mono),
          "Mod-Shift-7": restyle(STYLE.bulleted),
          "Mod-Shift-8": restyle(STYLE.dashed),
          "Mod-Shift-9": restyle(STYLE.numbered),
          "Mod-Shift-l": (s, d) => toggleChecklistCmd(s, d),
        }),
        keymap(baseKeymap),
      ],
    });
    const v = new EditorView(host.current, {
      state,
      nodeViews: { attachment: (node) => attachmentView(node) },
      editable: () => editable,
      attributes: {
        class: "note-rich",
        role: "textbox",
        "aria-multiline": "true",
        "aria-label": label,
        spellcheck: "true",
        ...(placeholder ? { "data-placeholder": placeholder } : {}),
      },
      dispatchTransaction(tr: Transaction) {
        const next = v.state.apply(tr);
        v.updateState(next);
        if (tr.docChanged) reportSoon();
        callbacks.current.onStatus?.(statusOf(next));
      },
      // A paste never brings an attachment's reference: Apple keeps each table or picture in
      // one place in one note, and a pasted copy referred to the same one twice, which made
      // Notes on a phone lag and crash. The copy's text (a table's cells) comes instead.
      // A pasted table, as Notes pastes one into text: a line per row, its cells by tabs.
      transformPastedHTML(html) {
        return html
          .replace(/<\/t[dh]>\s*<t[dh][^>]*>/gi, "\t")
          .replace(/<tr[^>]*>/gi, "<p>")
          .replace(/<\/tr>/gi, "</p>")
          .replace(/<\/?(table|tbody|thead|tfoot|td|th)[^>]*>/gi, "");
      },
      transformPasted(slice) {
        return new Slice(withoutAttachments(slice.content), slice.openStart, slice.openEnd);
      },
      // Lists typed as on the phone: "- " a dashed list, "* " a bulleted one, "1. " a numbered
      // one; in a dashed line "[ ] " or "[x] " makes it a checklist item, so "- [ ] " works
      // (icloud.com's notes3 userDidInsert). The typed marker goes; one undo brings it back.
      handleTextInput(view, from, to, text) {
        if (text !== " " || from !== to) return false;
        const $from = view.state.doc.resolve(from);
        const line = $from.parent;
        if (line.type !== schema.nodes.paragraph) return false;
        const typed = line.textBetween(0, $from.parentOffset, undefined, "\ufffc") + " ";
        const style = line.attrs.style as number | null;
        const inList = style !== null && LIST_STYLES.includes(style);
        let next: { style: number; checked: boolean | null; start?: number | null } | null = null;
        const numbered = /^([0-9]{1,6})\. $/.exec(typed);
        if (style === STYLE.dashed && (typed === "[ ] " || typed === "[x] ")) {
          next = { style: STYLE.checklist, checked: typed === "[x] " };
        } else if (!inList) {
          if (typed === "- ") next = { style: STYLE.dashed, checked: null };
          else if (typed === "* ") next = { style: STYLE.bulleted, checked: null };
          // "5. " starts the list at 5, as Notes keeps it.
          else if (numbered) next = { style: STYLE.numbered, checked: null, start: Number(numbered[1]) === 1 ? null : Number(numbered[1]) };
        }
        if (!next) return false;
        const tr = view.state.tr.delete($from.start(), from);
        tr.setNodeMarkup($from.before(), undefined, { ...line.attrs, style: next.style, checked: next.checked, start: next.start ?? null });
        view.dispatch(tr.scrollIntoView());
        return true;
      },
      handleDOMEvents: {
        click: (_vw, event) => openLinkAt(event),
        blur: () => {
          if (later.current !== null) report();
          callbacks.current.onBlur?.();
          return false;
        },
        // A checklist item's circle ticks it, as a tap does on the phone.
        mousedown: (vw, event) => {
          const line = (event.target as HTMLElement).closest?.('p[data-style="103"]') as HTMLElement | null;
          if (!line || !editable) return false;
          const box = line.getBoundingClientRect();
          const pad = parseFloat(getComputedStyle(line).paddingInlineStart) || 0;
          if (event.clientX - box.left > pad) return false;
          const pos = vw.posAtDOM(line, 0) - 1;
          const node = vw.state.doc.nodeAt(pos);
          if (!node) return false;
          event.preventDefault();
          vw.dispatch(vw.state.tr.setNodeMarkup(pos, undefined, { ...node.attrs, checked: !node.attrs.checked }));
          return true;
        },
      },
    });
    view.current = v;
    callbacks.current.onStatus?.(statusOf(v.state));
    if (autoFocus) {
      v.focus();
      v.dispatch(v.state.tr.setSelection(TextSelection.atEnd(v.state.doc)));
    }
    return () => {
      if (later.current !== null) report();
      v.destroy();
      view.current = null;
    };
    // One editor per note: the parent keys it by the note, and replace() takes later text.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    view.current?.setProps({ editable: () => editable });
  }, [editable]);

  useImperativeHandle(ref, () => ({
    setStyle: (style) => {
      const v = view.current;
      if (!v) return;
      restyle(style)(v.state, v.dispatch);
      v.focus();
    },
    toggleChecklist: () => {
      const v = view.current;
      if (!v) return;
      toggleChecklistCmd(v.state, v.dispatch);
      v.focus();
    },
    toggleMark: (mark) => {
      const v = view.current;
      if (!v) return;
      toggleMark(schema.marks[mark])(v.state, v.dispatch);
      v.focus();
    },
    replace: (nextText, nextRuns) => {
      const v = view.current;
      if (!v) return;
      const doc = toDoc(nextText, nextRuns);
      if (doc.eq(v.state.doc)) return;
      const { from, to } = v.state.selection;
      const tr = v.state.tr.replaceWith(0, v.state.doc.content.size, doc.content);
      const size = tr.doc.content.size;
      const near = (at: number) => tr.doc.resolve(Math.max(0, Math.min(at, size)));
      tr.setSelection(TextSelection.between(near(from), near(to)));
      tr.setMeta("addToHistory", false);
      // Taken in quietly: what Apple holds is not an edit to send back.
      const next = v.state.apply(tr);
      v.updateState(next);
      callbacks.current.onStatus?.(statusOf(next));
    },
    focus: () => view.current?.focus(),
    flushChange: () => {
      if (later.current !== null) report();
    },
  }));

  const [cellMenu, setCellMenu] = useState<CellMenu | null>(null);
  useEffect(() => {
    openCellMenu = setCellMenu;
    return () => {
      if (openCellMenu === setCellMenu) openCellMenu = null;
    };
  }, []);

  return (
    <>
      <div ref={host} className="note-rich-host" />
      {cellMenu && <RowMenu at={cellMenu.at} entries={cellMenu.entries} onClose={() => setCellMenu(null)} />}
    </>
  );
});

const toggleChecklistCmd: Command = (state, dispatch) => {
  const all = linesIn(state).every(({ node }) => node.attrs.style === STYLE.checklist);
  return chainCommands(restyle(all ? null : STYLE.checklist))(state, dispatch);
};
