/**
 * Whether the screen a component belongs to is the one showing. Screens are kept once opened
 * (coming back to one is instant, and finds it as it was left — its place, its selection), so
 * a screen out of sight is still mounted: its menus and sheets must not be drawn, and its keys
 * must not answer, while another screen is in front.
 */
import { createContext, useContext, useEffect, useRef } from "react";

export const ScreenActive = createContext(true);

export const useScreenActive = () => useContext(ScreenActive);

/** The same, as a ref, for listeners set up once and read when a key comes. */
export function useActiveRef() {
  const active = useScreenActive();
  const ref = useRef(active);
  ref.current = active;
  useEffect(() => {
    ref.current = active;
  }, [active]);
  return ref;
}
