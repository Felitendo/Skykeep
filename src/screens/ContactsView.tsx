import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, displayName, messageOf, type Contact } from "../lib/ipc";

const KIND_LABEL: Record<string, string> = {
  phone: "phone",
  email: "email",
  url: "link",
  address: "address",
};

/** Initials for the avatar, from whichever name fields exist. */
function initials(contact: Contact): string {
  const name = displayName(contact).trim();
  const parts = name.split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  // A number-only contact has no useful initial, so show a glyph instead of a digit.
  if (/^[+\d]/.test(parts[0])) return "#";
  return parts
    .slice(0, 2)
    .map((p) => [...p][0]?.toUpperCase() ?? "")
    .join("");
}

function relative(seconds: number | null): string {
  if (seconds === null) return "never";
  const ago = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  if (ago < 60) return "just now";
  if (ago < 3600) return `${Math.floor(ago / 60)} min ago`;
  if (ago < 86_400) return `${Math.floor(ago / 3600)} h ago`;
  return `${Math.floor(ago / 86_400)} d ago`;
}

export function ContactsView() {
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [search, setSearch] = useState("");
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  const [lastSynced, setLastSynced] = useState<number | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(async (query: string) => {
    const [found, synced] = await Promise.all([
      api.contactsList(query.trim() || null),
      api.contactsLastSynced(),
    ]);
    setContacts(Array.isArray(found) ? found : []);
    setLastSynced(typeof synced === "number" ? synced : null);
    setLoaded(true);
  }, []);

  const refreshRef = useRef<null | (() => Promise<void>)>(null);

  const refresh = useCallback(async () => {
    setSyncing(true);
    setError(null);
    try {
      await api.contactsSync();
      await read(search);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setSyncing(false);
    }
  }, [read, search]);
  refreshRef.current = refresh;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await read(search);
        if (cancelled) return;
        if ((await api.contactsLastSynced()) === null) void refreshRef.current?.();
      } catch (e) {
        if (!cancelled) {
          setError(messageOf(e));
          setLoaded(true);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // Re-reading on every keystroke is the point: the filter runs in SQLite.
  }, [read, search]);

  useEffect(() => {
    if (contacts.length === 0) return setOpenGuid(null);
    setOpenGuid((current) =>
      current && contacts.some((c) => c.guid === current) ? current : contacts[0].guid
    );
  }, [contacts]);

  const open = useMemo(
    () => contacts.find((c) => c.guid === openGuid) ?? null,
    [contacts, openGuid]
  );

  return (
    <div className="contacts">
      <section className="contacts-list" aria-label="Contacts">
        <header className="cal-bar">
          <div className="cal-nav">
            <h1>Contacts</h1>
            <span className="badge">{contacts.length}</span>
          </div>
          <div className="cal-actions">
            <span className="badge">{relative(lastSynced)}</span>
            <button className="quiet" onClick={refresh} disabled={syncing}>
              {syncing ? "Refreshing…" : "Refresh"}
            </button>
          </div>
        </header>

        <input
          className="contacts-search"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search"
          aria-label="Search contacts"
          type="search"
        />

        {error && <p className="error" role="alert">{error}</p>}

        <ul className="contact-rows">
          {contacts.map((contact) => (
            <li key={contact.guid}>
              <button
                className="contact-row"
                aria-current={openGuid === contact.guid}
                onClick={() => setOpenGuid(contact.guid)}
              >
                <span className="avatar" aria-hidden="true">{initials(contact)}</span>
                <span className="contact-row-name">{displayName(contact)}</span>
              </button>
            </li>
          ))}
        </ul>

        {loaded && contacts.length === 0 && !error && (
          <p className="cal-empty">{search ? "No matches." : "No contacts yet."}</p>
        )}
      </section>

      <article className="contact-detail" aria-label="Contact">
        {open ? (
          <>
            <span className="avatar large" aria-hidden="true">{initials(open)}</span>
            <h2>{displayName(open)}</h2>
            {open.company && !open.isCompany && <p className="contact-company">{open.company}</p>}

            <dl className="contact-methods">
              {open.methods.map((m, i) => (
                <div key={`${m.kind}-${i}`}>
                  <dt>{m.label?.toLowerCase() || KIND_LABEL[m.kind] || m.kind}</dt>
                  <dd>
                    {m.kind === "email" ? (
                      <a href={`mailto:${m.value}`}>{m.value}</a>
                    ) : m.kind === "phone" ? (
                      <a href={`tel:${m.value}`}>{m.value}</a>
                    ) : (
                      m.value
                    )}
                  </dd>
                </div>
              ))}
            </dl>

            {open.methods.length === 0 && (
              <p className="cal-empty">No phone numbers or addresses saved.</p>
            )}
            {open.notes && <p className="contact-notes">{open.notes}</p>}
          </>
        ) : (
          loaded && <p className="cal-empty">Select a contact.</p>
        )}
      </article>
    </div>
  );
}
