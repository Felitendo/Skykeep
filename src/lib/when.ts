/**
 * Dates and times written out in this machine's language, with each kind of format built
 * once and kept.
 *
 * `Date.toLocaleDateString(undefined, options)` builds a new formatter on every call, and a
 * formatter is not cheap — the locale's data is looked up and compiled each time. A calendar
 * week writes out hundreds of dates and times on every redraw, and those builds were most of
 * what made opening a new event stutter. A formatter made once per set of options and kept
 * gives the same text for a fraction of the cost.
 */
const made = new Map<string, Intl.DateTimeFormat>();

function formatter(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = JSON.stringify(options);
  let f = made.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(undefined, options);
    made.set(key, f);
  }
  return f;
}

/** As `date.toLocaleDateString(undefined, options)`, without building a formatter. */
export function dateText(date: Date, options: Intl.DateTimeFormatOptions): string {
  return formatter(options).format(date);
}

/** As `date.toLocaleTimeString(undefined, options)`, without building a formatter. */
export function timeText(date: Date, options: Intl.DateTimeFormatOptions): string {
  return formatter(options).format(date);
}
