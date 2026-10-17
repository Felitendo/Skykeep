/**
 * A note as Notes sets it: titles and headings, bulleted, dashed and numbered lists, checklists
 * with their circles, monospaced blocks, and bold, italic, underlined, struck and linked text —
 * read from Apple's own attribute runs, so what a phone or icloud.com formatted shows here
 * the same.
 *
 * A run's length counts UTF-16 units, which is how a JavaScript string counts, so the runs
 * lay over the text directly. Runs that do not add up to the text — a note edited here since
 * the last sync — are not guessed at: the caller shows the plain text instead.
 */
export type Run = {
  length: number;
  style: number | null;
  indent: number;
  checked: boolean | null;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  link: string | null;
};

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

/** Whether `runs` describe exactly this text. */
export function runsFit(text: string, runs: Run[]): boolean {
  return runs.length > 0 && runs.reduce((n, r) => n + r.length, 0) === text.length;
}

type Span = { text: string; run: Run };
type Paragraph = { spans: Span[]; run: Run; start: number };

/** The text cut into paragraphs, each with the style of the run its line ends in. */
function paragraphs(text: string, runs: Run[]): Paragraph[] {
  const out: Paragraph[] = [];
  let current: Paragraph = { spans: [], run: runs[0], start: 0 };
  let at = 0;
  for (const run of runs) {
    let piece = text.slice(at, at + run.length);
    let offset = at;
    at += run.length;
    while (piece.length > 0) {
      const nl = piece.indexOf("\n");
      const part = nl < 0 ? piece : piece.slice(0, nl);
      if (part) current.spans.push({ text: part, run });
      if (nl < 0) {
        current.run = run;
        break;
      }
      // The newline carries the paragraph's style, as Apple stores it.
      current.run = run;
      out.push(current);
      offset += nl + 1;
      current = { spans: [], run, start: offset };
      piece = piece.slice(nl + 1);
    }
  }
  if (current.spans.length > 0 || out.length === 0) out.push(current);
  return out;
}

function Inline({ span }: { span: Span }) {
  const { run, text } = span;
  let node: React.ReactNode = text;
  if (run.bold) node = <strong>{node}</strong>;
  if (run.italic) node = <em>{node}</em>;
  if (run.underline) node = <u>{node}</u>;
  if (run.strike) node = <s>{node}</s>;
  if (run.link) {
    node = (
      <a href={run.link} target="_blank" rel="noreferrer" onClick={(e) => e.stopPropagation()}>
        {node}
      </a>
    );
  }
  return <>{node}</>;
}

/**
 * The body below the title. `skip` is how much of the text the title line takes, so the
 * runs still line up with what is drawn.
 */
export function FormattedBody({
  text,
  runs,
  skip,
  onTick,
}: {
  text: string;
  runs: Run[];
  skip: number;
  onTick?: (paragraphStart: number, checked: boolean) => void;
}) {
  const all = paragraphs(text, runs).filter((p) => p.start >= skip);
  let number = 0;
  return (
    <div className="note-formatted">
      {all.map((p, i) => {
        const style = p.run?.style ?? STYLE.body;
        number = style === STYLE.numbered ? number + 1 : 0;
        const indent = { paddingInlineStart: `${(p.run?.indent ?? 0) * 1.5}em` };
        const content = p.spans.length ? p.spans.map((s, j) => <Inline key={j} span={s} />) : <br />;
        if (style === STYLE.title) return <h2 key={i} className="nf-title" style={indent}>{content}</h2>;
        if (style === STYLE.heading) return <h3 key={i} className="nf-heading" style={indent}>{content}</h3>;
        if (style === STYLE.subheading) return <h4 key={i} className="nf-subheading" style={indent}>{content}</h4>;
        if (style === STYLE.mono) return <pre key={i} className="nf-mono" style={indent}>{content}</pre>;
        if (style === STYLE.checklist) {
          const done = Boolean(p.run?.checked);
          return (
            <p key={i} className={`nf-item nf-check${done ? " is-done" : ""}`} style={indent}>
              <button
                type="button"
                className="nf-circle"
                role="checkbox"
                aria-checked={done}
                aria-label={p.spans.map((s) => s.text).join("") || "Checklist item"}
                disabled={!onTick}
                onClick={(e) => {
                  e.stopPropagation();
                  onTick?.(p.start, !done);
                }}
              />
              <span>{content}</span>
            </p>
          );
        }
        if (style === STYLE.bulleted || style === STYLE.dashed || style === STYLE.numbered) {
          const mark = style === STYLE.bulleted ? "•" : style === STYLE.dashed ? "–" : `${number}.`;
          return (
            <p key={i} className="nf-item" style={indent}>
              <span className="nf-mark" aria-hidden="true">{mark}</span>
              <span>{content}</span>
            </p>
          );
        }
        return <p key={i} className="nf-body" style={indent}>{content}</p>;
      })}
    </div>
  );
}
