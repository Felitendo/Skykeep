/**
 * A note as an editor holds it, and back: Apple's text and attribute runs turned into a
 * ProseMirror document of paragraphs — each with Notes' style, indent and checklist tick —
 * holding text with bold, italic, underline, strikethrough and links, and that document
 * turned back into exactly the text and runs Apple keeps.
 *
 * The round trip is lossless for everything the editor shows: a note read in and written
 * straight back gives the same text and the same runs. What it does not show — fonts,
 * colours, an attachment's insides — the backend keeps from Apple's own copy, so it is never
 * at stake here. An attachment's place in the text (U+FFFC) and a line break inside a
 * paragraph (U+2028, Shift-Return) travel as nodes of their own, so they cannot be typed
 * over by accident or lost.
 */
import { Schema, type Node as PMNode, type Mark } from "prosemirror-model";
import type { Run } from "./Formatted";

/** Apple's paragraph styles. */
export const STYLE = {
  title: 0,
  heading: 1,
  subheading: 2,
  body: 3,
  mono: 4,
  bulleted: 100,
  dashed: 101,
  numbered: 102,
  checklist: 103,
} as const;

export const LIST_STYLES: readonly number[] = [STYLE.bulleted, STYLE.dashed, STYLE.numbered, STYLE.checklist];
const ATTACHMENT = "￼";
const LINE_BREAK = " ";

export const schema = new Schema({
  nodes: {
    doc: { content: "paragraph+" },
    paragraph: {
      content: "inline*",
      attrs: {
        style: { default: null as number | null },
        indent: { default: 0 },
        checked: { default: null as boolean | null },
      },
      parseDOM: [{ tag: "p" }],
      toDOM: (node) => [
        "p",
        {
          class: "nf-p",
          "data-style": node.attrs.style === null ? "none" : String(node.attrs.style),
          ...(node.attrs.checked ? { "data-checked": "true" } : {}),
          style: node.attrs.indent ? `--indent: ${node.attrs.indent}` : "",
        },
        0,
      ],
    },
    text: { group: "inline" },
    /** Where an attachment sits — a picture, a sketch, a table — kept, not edited. */
    attachment: {
      group: "inline",
      inline: true,
      atom: true,
      selectable: true,
      attrs: { id: { default: null as string | null } },
      toDOM: () => ["span", { class: "nf-attachment", contenteditable: "false", title: "Attachment — open it on an Apple device" }, "Attachment"],
    },
    /** Shift-Return: a new line in the same paragraph. */
    line_break: {
      group: "inline",
      inline: true,
      selectable: false,
      parseDOM: [{ tag: "br" }],
      toDOM: () => ["br"],
    },
  },
  marks: {
    bold: { parseDOM: [{ tag: "strong" }, { tag: "b" }], toDOM: () => ["strong", 0] },
    italic: { parseDOM: [{ tag: "em" }, { tag: "i" }], toDOM: () => ["em", 0] },
    underline: { parseDOM: [{ tag: "u" }], toDOM: () => ["u", 0] },
    strike: { parseDOM: [{ tag: "s" }, { tag: "del" }], toDOM: () => ["s", 0] },
    link: {
      attrs: { href: {} },
      inclusive: false,
      parseDOM: [{ tag: "a[href]", getAttrs: (d) => ({ href: (d as HTMLElement).getAttribute("href") }) }],
      toDOM: (m) => ["a", { href: m.attrs.href, rel: "noreferrer", target: "_blank" }, 0],
    },
  },
});

const plain = (length: number): Run => ({
  length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null,
});

/** Whether `runs` describe exactly this text. */
export function runsFit(text: string, runs: Run[] | null): runs is Run[] {
  return !!runs && runs.length > 0 && runs.reduce((n, r) => n + r.length, 0) === text.length;
}

/** The run each UTF-16 unit of the text lies in. */
function perUnit(text: string, runs: Run[]): Run[] {
  const out: Run[] = [];
  for (const r of runs) for (let i = 0; i < r.length; i++) out.push(r);
  while (out.length < text.length) out.push(runs[runs.length - 1] ?? plain(1));
  return out;
}

function marksOf(run: Run): Mark[] {
  const m: Mark[] = [];
  if (run.bold) m.push(schema.marks.bold.create());
  if (run.italic) m.push(schema.marks.italic.create());
  if (run.underline) m.push(schema.marks.underline.create());
  if (run.strike) m.push(schema.marks.strike.create());
  if (run.link) m.push(schema.marks.link.create({ href: run.link }));
  return m;
}

/** Apple's text and runs as a document. Without runs that fit, every line is plain body. */
export function toDoc(text: string, runs: Run[] | null): PMNode {
  const fits = runsFit(text, runs);
  const units = fits ? perUnit(text, runs) : [];
  const paragraphs: PMNode[] = [];
  let start = 0;
  for (;;) {
    const nl = text.indexOf("\n", start);
    const end = nl < 0 ? text.length : nl;
    // A paragraph is set by the run its newline is in — or, for the last line, its last unit.
    const own = fits ? (units[nl < 0 ? Math.max(end - 1, start) : nl] ?? units[units.length - 1]) : null;
    const inline: PMNode[] = [];
    let i = start;
    while (i < end) {
      const ch = text[i];
      if (ch === ATTACHMENT) {
        inline.push(schema.nodes.attachment.create({ id: fits ? (units[i]?.attachment ?? null) : null }));
        i += 1;
        continue;
      }
      if (ch === LINE_BREAK) {
        inline.push(schema.nodes.line_break.create());
        i += 1;
        continue;
      }
      let j = i + 1;
      while (j < end && text[j] !== ATTACHMENT && text[j] !== LINE_BREAK && (!fits || units[j] === units[i])) j++;
      inline.push(schema.text(text.slice(i, j), fits ? marksOf(units[i]) : []));
      i = j;
    }
    paragraphs.push(
      schema.nodes.paragraph.create(
        {
          style: own?.style ?? null,
          indent: own?.indent ?? 0,
          checked: own?.style === STYLE.checklist ? Boolean(own.checked) : null,
        },
        inline,
      ),
    );
    if (nl < 0) break;
    start = nl + 1;
  }
  return schema.nodes.doc.create(null, paragraphs);
}

const same = (a: Run, b: Run) =>
  a.style === b.style && a.indent === b.indent && a.checked === b.checked && a.bold === b.bold &&
  a.italic === b.italic && a.underline === b.underline && a.strike === b.strike && a.link === b.link &&
  (a.attachment ?? null) === (b.attachment ?? null);

/** A document as Apple's text and runs. */
export function fromDoc(doc: PMNode): { text: string; runs: Run[] } {
  let text = "";
  const runs: Run[] = [];
  const push = (piece: string, run: Run) => {
    if (!piece) return;
    text += piece;
    const last = runs[runs.length - 1];
    if (last && same(last, run)) last.length += piece.length;
    else runs.push({ ...run, length: piece.length });
  };
  doc.forEach((para, _offset, index) => {
    const p = {
      style: para.attrs.style as number | null,
      indent: para.attrs.indent as number,
      checked: para.attrs.style === STYLE.checklist ? Boolean(para.attrs.checked) : null,
    };
    const base: Run = { ...plain(0), ...p };
    para.forEach((child) => {
      if (child.type === schema.nodes.attachment) return push(ATTACHMENT, { ...base, attachment: child.attrs.id ?? null });
      if (child.type === schema.nodes.line_break) return push(LINE_BREAK, base);
      const has = (name: string) => child.marks.some((m) => m.type.name === name);
      push(child.text ?? "", {
        ...base,
        bold: has("bold"),
        italic: has("italic"),
        underline: has("underline"),
        strike: has("strike"),
        link: (child.marks.find((m) => m.type.name === "link")?.attrs.href as string | undefined) ?? null,
      });
    });
    // The newline carries the paragraph's setting, as Apple stores it.
    if (index < doc.childCount - 1) push("\n", base);
  });
  return { text, runs };
}

/** The one stretch `to` changed from `from`, in UTF-16 units: where, removed, inserted. */
function change(from: string, to: string): [number, number, number] {
  let pre = 0;
  const max = Math.min(from.length, to.length);
  while (pre < max && from[pre] === to[pre]) pre++;
  let suf = 0;
  while (suf < max - pre && from[from.length - 1 - suf] === to[to.length - 1 - suf]) suf++;
  return [pre, from.length - pre - suf, to.length - pre - suf];
}

/**
 * `runs`, which describe `from`, laid over `to` — the same text with another device's edit
 * merged in. What is new takes the setting of what is just before it. The backend does the
 * same with the same rule, so the screen and Apple agree.
 */
export function carryRuns(from: string, runs: Run[], to: string): Run[] {
  if (!runsFit(from, runs)) return [plain(to.length)];
  const units = perUnit(from, runs);
  const [at, removed, inserted] = change(from, to);
  const fill = at > 0 ? units[at - 1] : (units[at + removed] ?? plain(1));
  const next = [...units.slice(0, at), ...Array<Run>(inserted).fill(fill), ...units.slice(at + removed)];
  const out: Run[] = [];
  for (const r of next) {
    const last = out[out.length - 1];
    if (last && same(last, r)) last.length += 1;
    else out.push({ ...r, length: 1 });
  }
  return out;
}
