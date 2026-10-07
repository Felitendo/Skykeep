import { useCallback, useEffect, useMemo, useState } from "react";
import { Notice } from "../components/Notice";
import { ChevronLeft } from "lucide-react";
import {
  api,
  displayName,
  messageOf,
  photoSrc,
  type Contact,
  type ContactMethod,
} from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Picker } from "../components/Picker";

/** The kinds of detail a card can hold, in the order Apple lists them. */
const KINDS = [
  { value: "phone", label: "phone" },
  { value: "email", label: "email" },
  { value: "url", label: "link" },
] as const;

/**
 * A contact's picture where there is one, initials where there is not.
 *
 * The image is allowed to fail — a photo Apple has moved, or no network yet — and when it
 * does the initials underneath are what shows, rather than a broken frame.
 */
function Avatar({ contact, large }: { contact: Contact; large?: boolean }) {
  const [failed, setFailed] = useState(false);
  const src = failed ? null : photoSrc(contact);
  return (
    <span className={`avatar${large ? " large" : ""}`} aria-hidden="true">
      {src ? (
        <img src={src} alt="" onError={() => setFailed(true)} />
      ) : (
        initials(contact)
      )}
    </span>
  );
}

const blank = (): Contact => ({
  guid: "",
  firstName: null,
  middleName: null,
  photoUrl: null,
  photoSignature: null,
  lastName: null,
  company: null,
  isCompany: false,
  birthday: null,
  notes: null,
  etag: null,
  methods: [],
});

const KIND_LABEL: Record<string, string> = {
  phone: "phone",
  email: "email",
  url: "link",
  address: "address",
  profile: "profile",
};

/** What this app can safely write back. Apple stores an address as a structured thing and a
 *  profile as an encoded handle; editing either as one line here would mangle it, so they
 *  are shown and left alone rather than half-supported. */
const EDITABLE_KINDS = new Set(["phone", "email", "url"]);

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
  /** In a narrow window, the person picked rather than the list — as on iPhone. */
  const [detail, setDetail] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { revision } = useSync();
  /** The card being written, or null while simply looking. */
  const [draft, setDraft] = useState<Contact | null>(null);
  const [busy, setBusy] = useState(false);

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

  /** Every write goes to Apple first, then the library is re-read. */
  const write = useCallback(
    async (run: () => Promise<void>) => {
      setBusy(true);
      setError(null);
      try {
        await run();
        await read(search);
        return true;
      } catch (e) {
        setError(messageOf(e));
        return false;
      } finally {
        setBusy(false);
      }
    },
    [read, search]
  );

  const saveDraft = useCallback(async () => {
    if (!draft) return;
    const saving = draft;
    let guid = saving.guid;
    const ok = await write(async () => {
      guid = await api.contactSave(saving);
    });
    if (ok) {
      setDraft(null);
      setOpenGuid(guid);
    }
  }, [draft, write]);

  const removeContact = useCallback(
    async (guid: string) => {
      if (await write(() => api.contactDelete(guid))) setDraft(null);
    },
    [write]
  );

  /** Editing a method in the draft, by position. */
  const editMethod = (index: number, patch: Partial<ContactMethod>) =>
    setDraft((d) =>
      d
        ? {
            ...d,
            methods: d.methods.map((m, i) => (i === index ? { ...m, ...patch } : m)),
          }
        : d
    );

  const open = useMemo(
    () => contacts.find((c) => c.guid === openGuid) ?? null,
    [contacts, openGuid]
  );

  return (
    // A contact being written is always what shows; it has its own Cancel.
    <div className={`contacts${detail || draft ? " is-detail" : ""}`}>
      <section className="contacts-list" aria-label="Contacts">
        <header className="cal-bar">
          <div className="cal-nav">
            <h1>Contacts</h1>
            <span className="badge">{contacts.length}</span>
          </div>
          <div className="cal-actions">
            <button className="quiet" disabled={busy} onClick={() => setDraft(blank())}>
              New contact
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

        {error && <Notice message={error} onDismiss={() => setError(null)} />}

        <ul className="contact-rows">
          {contacts.map((contact) => (
            <li key={contact.guid}>
              <button
                className="contact-row"
                aria-current={openGuid === contact.guid}
                onClick={() => {
                  setOpenGuid(contact.guid);
                  setDetail(true);
                }}
              >
                <Avatar contact={contact} />
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
        {!draft && (
          <button className="compact-back" onClick={() => setDetail(false)}>
            <ChevronLeft size={18} strokeWidth={2.25} aria-hidden="true" />
            <span>Contacts</span>
          </button>
        )}
        {draft ? (
          <form
            className="contact-form"
            onSubmit={(e) => {
              e.preventDefault();
              void saveDraft();
            }}
          >
            <h2>{draft.guid ? "Edit contact" : "New contact"}</h2>

            <div className="contact-names">
              <label>
                <span>First</span>
                <input
                  value={draft.firstName ?? ""}
                  autoFocus
                  onChange={(e) => setDraft({ ...draft, firstName: e.target.value || null })}
                />
              </label>
              <label>
                <span>Last</span>
                <input
                  value={draft.lastName ?? ""}
                  onChange={(e) => setDraft({ ...draft, lastName: e.target.value || null })}
                />
              </label>
              <label>
                <span>Company</span>
                <input
                  value={draft.company ?? ""}
                  onChange={(e) => setDraft({ ...draft, company: e.target.value || null })}
                />
              </label>
            </div>

            <ul className="contact-method-rows">
              {draft.methods.map((m, index) =>
                !EDITABLE_KINDS.has(m.kind) ? (
                  // Shown so it is not a surprise that it exists, but not offered for
                  // editing — this app cannot write a structured address back safely.
                  <li key={index} className="contact-method-fixed">
                    <span>{KIND_LABEL[m.kind] ?? m.kind}</span>
                    <span className="contact-address">{m.value}</span>
                    <span className="badge">edited on an Apple device</span>
                  </li>
                ) : (
                <li key={index}>
                  <Picker
                    label={`Kind of detail ${index + 1}`}
                    className="contact-kind"
                    value={m.kind}
                    onChange={(kind) => editMethod(index, { kind })}
                    options={KINDS.map((k) => ({ value: k.value, label: k.label }))}
                  />
                  <input
                    className="contact-method-label"
                    value={m.label ?? ""}
                    placeholder="label"
                    aria-label={`Label for detail ${index + 1}`}
                    onChange={(e) => editMethod(index, { label: e.target.value || null })}
                  />
                  <input
                    className="contact-method-value"
                    value={m.value}
                    placeholder={m.kind === "email" ? "name@example.com" : "+1 555 0100"}
                    aria-label={`Value for detail ${index + 1}`}
                    onChange={(e) => editMethod(index, { value: e.target.value })}
                  />
                  <button
                    type="button"
                    className="quiet danger"
                    aria-label={`Remove detail ${index + 1}`}
                    onClick={() =>
                      setDraft({
                        ...draft,
                        methods: draft.methods.filter((_, i) => i !== index),
                      })
                    }
                  >
                    ✕
                  </button>
                </li>
                ),
              )}
            </ul>

            <button
              type="button"
              className="quiet"
              onClick={() =>
                setDraft({
                  ...draft,
                  methods: [...draft.methods, { kind: "phone", label: null, value: "" }],
                })
              }
            >
              Add phone, email or link
            </button>

            <label className="contact-notes-field">
              <span>Notes</span>
              <textarea
                rows={3}
                value={draft.notes ?? ""}
                onChange={(e) => setDraft({ ...draft, notes: e.target.value || null })}
              />
            </label>

            <div className="cal-form-actions">
              <button
                type="submit"
                className="primary"
                disabled={
                  busy || !(draft.firstName || draft.lastName || draft.company)
                }
              >
                {busy ? "Saving…" : draft.guid ? "Save" : "Add contact"}
              </button>
              <button type="button" className="quiet" onClick={() => setDraft(null)}>
                Cancel
              </button>
            </div>
          </form>
        ) : open ? (
          <>
            {/* Photo and name together at the top of the card, as Contacts arranges them.
                The avatar used to be pinned to the pane's left edge while the name was
                centred somewhere else entirely, so they read as two unrelated things. */}
            <div className="contact-head">
              <Avatar contact={open} large />
              <div className="contact-head-names">
                <h2>{displayName(open)}</h2>
                {open.company && !open.isCompany && (
                  <p className="contact-company">{open.company}</p>
                )}
              </div>
            </div>

            <dl className="contact-methods">
              {open.methods.map((m, i) => (
                <div key={`${m.kind}-${i}`}>
                  <dt>{m.label?.toLowerCase() || KIND_LABEL[m.kind] || m.kind}</dt>
                  <dd>
                    {m.kind === "email" ? (
                      <a href={`mailto:${m.value}`}>{m.value}</a>
                    ) : m.kind === "phone" ? (
                      <a href={`tel:${m.value}`}>{m.value}</a>
                    ) : m.kind === "address" ? (
                      // A postal address is several lines and stays that way.
                      <span className="contact-address">{m.value}</span>
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

            <div className="cal-form-actions">
              <button className="quiet" disabled={busy} onClick={() => setDraft({ ...open })}>
                Edit
              </button>
              <button
                className="quiet danger"
                disabled={busy}
                onClick={() => {
                  // Deleting a person is not something to discover by accident.
                  if (window.confirm(`Delete ${displayName(open)}?`)) {
                    void removeContact(open.guid);
                  }
                }}
              >
                Delete
              </button>
            </div>
          </>
        ) : (
          loaded && <p className="cal-empty">Select a contact.</p>
        )}
      </article>
    </div>
  );
}
