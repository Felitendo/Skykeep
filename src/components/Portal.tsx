import { createPortal } from "react-dom";

/**
 * Menus, popovers and sheets are drawn at the top of the page, over everything, as a
 * window server draws them. Inside a screen, any ancestor with a transform, filter or
 * backdrop blur becomes what `position: fixed` is fixed to: a menu opened from Drive's grid
 * came out the width of the screen, and File Info sat in a corner instead of the middle.
 */
export function Portal({ children }: { children: React.ReactNode }) {
  return createPortal(children, document.body);
}

/**
 * Whether a press landed in a menu drawn in this top layer. A popover or sheet closes on a
 * press outside itself, but a menu it opened is drawn out here, not inside it: choosing
 * from that menu read as a press elsewhere and closed the popover with the choice.
 */
export function inMenu(target: EventTarget | null): boolean {
  return Boolean((target as HTMLElement | null)?.closest?.(".picker-menu, .row-menu"));
}
