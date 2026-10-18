import "prosemirror-view/style/prosemirror.css";
import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import { EditorState, TextSelection, type Command, type Transaction } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { history, undo, redo } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import { baseKeymap, splitBlockAs, toggleMark, chainCommands } from "prosemirror-commands";
import type { Node as PMNode } from "prosemirror-model";
import type { Run } from "./Formatted";
import { fromDoc, LIST_STYLES, schema, STYLE, toDoc } from "./rich";
import { api } from "../../lib/ipc";

/**
 * An attachment in the note: a table drawn as the grid it is, with the cells Apple holds;
 * anything else — a picture, a sketch — as a labelled placeholder that keeps its place. Not
 * editable here, and never written from here: the note keeps the attachment exactly.
 */
function attachmentView(node: PMNode) {
  const dom = document.createElement("span");
  dom.className = "nf-attachment";
  dom.contentEditable = "false";
  dom.textContent = "Attachment";
  dom.title = "Open it on an Apple device to change it";
  const id = node.attrs.id as string | null;
  if (id) {
    api.noteTable(id).then(
      (cells) => {
        if (!cells || !cells.length) return;
        const table = document.createElement("table");
        table.className = "nf-table";
        for (const row of cells) {
          const tr = table.insertRow();
          for (const cell of row) tr.insertCell().textContent = cell.trimEnd();
        }
        dom.className = "nf-attachment is-table";
        dom.title = "A table — edit it on an Apple device for now";
        dom.replaceChildren(table);
      },
      () => {},
    );
  }
  return { dom, ignoreMutation: () => true, stopEvent: () => false };
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
      return { type: schema.nodes.paragraph, attrs: { ...node.attrs, checked: style === STYLE.checklist ? false : null } };
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
      handleDOMEvents: {
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

  return <div ref={host} className="note-rich-host" />;
});

const toggleChecklistCmd: Command = (state, dispatch) => {
  const all = linesIn(state).every(({ node }) => node.attrs.style === STYLE.checklist);
  return chainCommands(restyle(all ? null : STYLE.checklist))(state, dispatch);
};
