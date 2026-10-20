/**
 * Shared Albums, as icloud.com's Sharing section lists them: each a card — made by this account
 * ("Shared by you") or joined ("Shared with you") — read from Apple's own service each time the
 * page opens. A new one is named in a sheet and is on the page at once, then confirmed by Apple
 * (or taken back, with Apple's reason); one of this account's is deleted from its menu, asked
 * first, for everyone it was shared with.
 */
import { useEffect, useRef, useState } from "react";
import { Plus, Users } from "lucide-react";
import { api, type SharedAlbum } from "../../lib/ipc";
import { Portal } from "../../components/Portal";
import { RowMenu, type MenuEntry } from "../../components/RowMenu";
import type { Question } from "./Ask";

export function SharedAlbums({ ask, onError }: { ask: (q: Question) => Promise<boolean>; onError: (message: string) => void }) {
  const [albums, setAlbums] = useState<SharedAlbum[] | null>(null);
  const [naming, setNaming] = useState(false);
  const [menu, setMenu] = useState<{ at: { x: number; y: number; alignRight: boolean }; entries: MenuEntry[] } | null>(null);

  const read = () =>
    api.photosSharedAlbums().then(setAlbums, (e) => {
      setAlbums([]);
      onError(String(e));
    });
  useEffect(() => {
    void read();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const create = async (name: string) => {
    setNaming(false);
    const temp: SharedAlbum = { guid: `new:${Date.now()}`, name, createdAt: Math.floor(Date.now() / 1000), owned: true, public: false, contributions: true };
    setAlbums((list) => [temp, ...(list ?? [])]);
    try {
      const guid = await api.photosSharedCreate(name);
      setAlbums((list) => (list ?? []).map((a) => (a.guid === temp.guid ? { ...a, guid } : a)));
    } catch (e) {
      setAlbums((list) => (list ?? []).filter((a) => a.guid !== temp.guid));
      onError(String(e));
    }
  };
  const remove = async (a: SharedAlbum) => {
    const sure = await ask({
      title: `Delete the shared album “${a.name}”?`,
      body: "It will be deleted from iCloud for everyone it is shared with, along with its photos and comments.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (!sure) return;
    const before = albums;
    setAlbums((list) => (list ?? []).filter((x) => x.guid !== a.guid));
    try {
      await api.photosSharedDelete(a.guid);
    } catch (e) {
      setAlbums(before);
      onError(String(e));
    }
  };

  const n = albums?.length ?? 0;
  return (
    <div className="px-albums-page">
      <header className="px-albums-head">
        <div className="px-heading">
          <h1>Shared Albums</h1>
          <p className="px-span">{albums === null ? " " : n === 1 ? "1 Item" : `${n} Items`}</p>
        </div>
        <div className="px-capsule">
          <button className="px-cap" aria-label="New Shared Album" title="New Shared Album" onClick={() => setNaming(true)}>
            <Plus size={16} strokeWidth={2} aria-hidden="true" />
          </button>
        </div>
      </header>
      {albums !== null && n === 0 ? (
        <div className="px-empty px-shared-empty">
          <p>No shared albums yet. Make one to share photos with the people you choose.</p>
          <button className="primary" onClick={() => setNaming(true)}>
            New Shared Album
          </button>
        </div>
      ) : (
        <div className="px-cards" role="list" aria-busy={albums === null}>
          {(albums ?? []).map((a) => (
            <div
              key={a.guid}
              role="listitem"
              className="px-card"
              aria-label={a.name}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({
                  at: { x: e.clientX, y: e.clientY, alignRight: false },
                  entries: a.owned ? [{ label: "Delete Shared Album", danger: true, run: () => void remove(a) }] : [{ label: "Shared with you — only its owner can delete it" }],
                });
              }}
            >
              <span className="px-card-pic">
                <Users size={34} strokeWidth={1.6} aria-hidden="true" />
              </span>
              <span className="px-card-name">{a.name}</span>
              <span className="px-card-count">
                {a.owned ? "Shared by you" : "Shared with you"}
                {a.createdAt ? ` · ${new Date(a.createdAt * 1000).toLocaleDateString(undefined, { dateStyle: "medium" })}` : ""}
              </span>
            </div>
          ))}
        </div>
      )}
      {naming && <NameSheet onCancel={() => setNaming(false)} onCreate={(name) => void create(name)} />}
      {menu && <RowMenu at={menu.at} entries={menu.entries} onClose={() => setMenu(null)} />}
    </div>
  );
}

/** The name for a new shared album, asked in a sheet: Return makes it, Escape leaves it. */
function NameSheet({ onCancel, onCreate }: { onCancel: () => void; onCreate: (name: string) => void }) {
  const [name, setName] = useState("");
  const field = useRef<HTMLInputElement | null>(null);
  useEffect(() => field.current?.focus(), []);
  const go = () => name.trim() && onCreate(name.trim());
  return (
    <Portal>
      <div className="modal-veil" onClick={onCancel}>
        <form
          className="modal-sheet px-ask"
          role="dialog"
          aria-label="New Shared Album"
          onClick={(e) => e.stopPropagation()}
          onSubmit={(e) => {
            e.preventDefault();
            go();
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              onCancel();
            }
          }}
        >
          <h2>New Shared Album</h2>
          <input ref={field} className="px-shared-name" placeholder="Album Name" aria-label="Album Name" value={name} onChange={(e) => setName(e.target.value)} />
          <div className="px-ask-actions">
            <button type="button" onClick={onCancel}>
              Cancel
            </button>
            <button type="submit" className="is-act" disabled={!name.trim()}>
              Create
            </button>
          </div>
        </form>
      </div>
    </Portal>
  );
}
