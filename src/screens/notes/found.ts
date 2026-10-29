/**
 * Where the words of a search fall in a piece of text, as Notes marks them in its list and in
 * the note opened from it: each word typed matches the start of a word, case and accents aside
 * — the way the library's index found the note (search.rs), so what is marked is what found it.
 */

/** A character as the search compares it: lower case, its accents off. */
const foldChar = (c: string) => c.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

/** The words of a query, folded, longest first — so "spring" marks all of "Springfield". */
export function queryWords(query: string): string[] {
  return [...new Set(query.split(/\s+/).map((w) => [...w].map(foldChar).join("").replace(/[^\p{L}\p{N}]/gu, "")).filter(Boolean))]
    .sort((a, b) => b.length - a.length);
}

const WORD = /[\p{L}\p{N}\p{M}]+/gu;

/** Each place in `text` a word begins with one of `words`, as [from, to) in UTF-16 units. */
export function foundRanges(text: string, words: string[]): [number, number][] {
  if (!words.length || !text) return [];
  const out: [number, number][] = [];
  for (const m of text.matchAll(WORD)) {
    const word = m[0];
    const at = m.index ?? 0;
    for (const w of words) {
      // Folded a character at a time, so the end of the match is known in the text itself.
      let folded = "";
      let end = 0;
      for (const c of word) {
        if (folded.length >= w.length) break;
        folded += foldChar(c);
        end += c.length;
      }
      if (folded.startsWith(w)) {
        out.push([at, at + end]);
        break;
      }
    }
  }
  return out;
}
