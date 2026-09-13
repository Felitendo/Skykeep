import { useCallback, useEffect, useMemo, useState } from "react";
import { api, displayName, messageOf, type Contact } from "../lib/ipc";
import { useSync } from "../lib/sync";

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

export function ContactsView() {
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [search, setSearch] = useState("");
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { revision } = useSync();

  const read = useCallback(async (query: string) => {
    const found = await api.contactsList(query.trim() || null);
    setContacts(Array.isArray(found) ? found : []);
    setLoaded(true);
  }, []);

  // The library is the only thing this screen reads. `revision` ticks whenever a refresh
  // finishes — pressed or on the timer — so a change made on the phone lands here without
  // this screen knowing anything about the network.
  useEffect(() => {
    let cancelled = false;
    read(search).catch((e) => {
      if (!cancelled) {
        setError(messageOf(e));
        setLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
    // Re-reading on every keystroke is the point: the filter runs in SQLite.
  }, [read, search, revision]);

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
