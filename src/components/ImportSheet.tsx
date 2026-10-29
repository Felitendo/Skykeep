import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { intakeApi, INTAKE, type Arrival } from "../lib/ipc";

const ArrivalSheet = lazy(() => import("./ArrivalSheet"));

/**
 * A calendar or contacts file opened with Skykeep (src-tauri/src/intake.rs): what it holds,
 * read on this computer, and one button that adds it. Nothing reaches iCloud until then, and
 * what can't be added is listed with why, rather than left out unsaid.
 *
 * Mounted once per window; it shows the files that arrived for this window, one at a time.
 */
export function ImportSheet() {
  // Each file with an id of its own, given as it arrives: the sheet is keyed on it and put
  // away by it. Keyed on the queue's length, a second file arriving mid-import remade the
  // sheet with Add live again (the events added twice); put away as "the first", a finished
  // import whose sheet had already gone dropped the next file unseen.
  const [queue, setQueue] = useState<{ id: number; arrival: Arrival }[]>([]);
  const nextId = useRef(0);

  const collect = useCallback(() => {
    intakeApi
      .pending()
      .then((more) => more.length && setQueue((q) => [...q, ...more.map((arrival) => ({ id: nextId.current++, arrival }))]))
      .catch(() => {});
  }, []);

  useEffect(() => {
    collect();
    const stop = listen(INTAKE, collect);
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [collect]);

  const head = queue[0];
  if (!head) return null;
  const current = head.arrival;
  const done = () => setQueue((q) => q.filter((x) => x.id !== head.id));
  return (
    <Suspense fallback={null}>
      <ArrivalSheet key={head.id} arrival={current} onDone={done} />
    </Suspense>
  );
}
