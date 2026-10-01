import icon from "../../src-tauri/icons/icon.svg";

/**
 * Quince's own icon — the same tile the dock, the tray and every alert show — so the first
 * screen someone sees is the app they will find later, not a separate line drawing of it.
 */
export function Mark({ className }: { className?: string }) {
  return <img className={className} src={icon} alt="" aria-hidden="true" draggable={false} />;
}
