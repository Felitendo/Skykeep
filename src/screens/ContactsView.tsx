import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Notice } from "../components/Notice";
import { ChevronLeft, Plus, UsersRound, UserRound } from "lucide-react";
import {
  api,
  displayName,
  messageOf,
  photoSrc,
  type Contact,
  type ContactList,
  type ContactMethod,
} from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Picker } from "../components/Picker";
import { RowMenu, type MenuEntry } from "../components/RowMenu";
import { Portal } from "../components/Portal";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

/** The sidebar row that is every contact rather than a list. */
const ALL = "";

type MenuAt = { x: number; y: number; alignRight: boolean };

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
  const [lists, setLists] = useState<ContactList[]>([]);
  /** The list being looked at, or ALL. */
  const [list, setList] = useState<string>(ALL);
  /** A list whose name is being typed: "new" for one not made yet. */
  const [naming, setNaming] = useState<string | null>(null);
  const [listMenu, setListMenu] = useState<(MenuAt & { list: ContactList }) | null>(null);
  /** A contact's menu: its actions, or — one step in — the lists it can go into. */
  const [contactMenu, setContactMenu] = useState<(MenuAt & { contact: Contact; lists: boolean }) | null>(null);
  /** The card that is this person's own. */
  const [me, setMe] = useState<string | null>(null);
  /** A moment's word that something finished — a file saved, cards brought in. */
  const [done, setDone] = useState<string | null>(null);
  const [moreMenu, setMoreMenu] = useState<MenuAt | null>(null);
  /** A contact being deleted while a list is open: out of the list, or gone everywhere? */
  const [deleting, setDeleting] = useState<Contact | null>(null);
  /** The people being dragged onto a list — one, or everyone selected. */
  const [dragging, setDragging] = useState<string[] | null>(null);
  /** More than one person selected, as Command- and Shift-click select in Contacts. */
  const [picked, setPicked] = useState<string[]>([]);
  const [pickMenu, setPickMenu] = useState<MenuAt | null>(null);
  const [dropOn, setDropOn] = useState<string | null>(null);
  /** List writes go to Apple one after another, in the order they were made. */
  const queue = useRef<Promise<unknown>>(Promise.resolve());

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

  const readLists = useCallback(async () => {
    const [found, mine] = await Promise.all([api.contactLists(), api.contactMe()]);
    setLists(Array.isArray(found) ? found : []);
    setMe(typeof mine === "string" ? mine : null);
  }, []);
  // The word is a moment, not something that lingers over the next thing done.
  useEffect(() => {
    if (!done) return undefined;
    const timer = window.setTimeout(() => setDone(null), 4000);
    return () => window.clearTimeout(timer);
  }, [done]);

  /** People out as a vCard, named as icloud.com names the file. */
  const exportCards = async (people: Contact[], label?: string) => {
    if (people.length === 0) return;
    const name =
      people.length === 1 ? displayName(people[0])
      : people.length === 2 ? `${displayName(people[0])} and ${displayName(people[1])}`
      : label ?? "iCloud vCards";
    try {
      const path = await api.contactsExport(people.map((c) => c.guid), `${name}.vcf`);
      setDone(`Saved to ${path}`);
    } catch (e) {
      setError(messageOf(e));
    }
  };
  const importCards = async () => {
    try {
      const chosen = await openFileDialog({ multiple: false, title: "Import vCard", filters: [{ name: "vCard", extensions: ["vcf", "vcard"] }] });
      if (!chosen || Array.isArray(chosen)) return;
      setBusy(true);
      const [added, failed] = await api.contactsImport(chosen);
      await Promise.all([read(search), readLists()]);
      setDone(`${added} ${added === 1 ? "contact" : "contacts"} imported${failed ? `, ${failed} could not be read` : ""}.`);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  };
  const makeMine = async (c: Contact) => {
    const was = me;
    setMe(c.guid);
    try {
      await api.contactSetMe(c.guid);
    } catch (e) {
      setMe(was);
      setError(messageOf(e));
    }
  };
  useEffect(() => {
    readLists().catch((e) => setError(messageOf(e)));
  }, [readLists, revision]);
  // A list deleted elsewhere is not left open, empty, with nothing to say why.
  useEffect(() => {
    if (list !== ALL && !lists.some((l) => l.guid === list)) setList(ALL);
  }, [lists, list]);

  /** Who shows: everyone, or the people in the open list. */
  const shown = useMemo(() => {
    if (list === ALL) return contacts;
    const members = new Set((lists.find((l) => l.guid === list)?.members ?? []).map((m) => m.toUpperCase()));
    return contacts.filter((c) => members.has(c.guid.toUpperCase()));
  }, [contacts, lists, list]);

  useEffect(() => {
    if (shown.length === 0) return setOpenGuid(null);
    setOpenGuid((current) =>
      current && shown.some((c) => c.guid === current) ? current : sortedContacts(shown)[0].guid
    );
  }, [shown]);

  /**
   * A change to a list: drawn at once, then sent. When Apple says no, the lists are read
   * back as Apple holds them, so nothing stays on screen that did not happen.
   */
  const changeList = useCallback(
    (predict: (lists: ContactList[]) => ContactList[], change: Parameters<typeof api.contactListWrite>[0]) => {
      setLists(predict);
      setError(null);
      const sent = queue.current.then(async () => {
        try {
          const guid = await api.contactListWrite(change);
          await readLists();
          return guid;
        } catch (e) {
          setError(messageOf(e));
          await readLists().catch(() => {});
          return null;
        }
      });
      queue.current = sent;
      return sent;
    },
    [readLists]
  );

  const createList = useCallback(
    async (name: string, members: string[] = []) => {
      const title = name.trim() || "Untitled List";
      const standIn = `pending-list-${Date.now()}`;
      const guid = await changeList(
        (ls) => [...ls, { guid: standIn, name: title, members, etag: null }],
        { name: title, add: members }
      );
      if (guid) setList((current) => (current === standIn ? guid : current));
      else setList((current) => (current === standIn ? ALL : current));
    },
    [changeList]
  );
  const renameList = (l: ContactList, name: string) => {
    setNaming(null);
    const title = name.trim();
    if (!title || title === l.name) return;
    void changeList((ls) => ls.map((x) => (x.guid === l.guid ? { ...x, name: title } : x)), { guid: l.guid, name: title });
  };
  const deleteList = (l: ContactList) => {
    // icloud.com's own words for it.
    if (!window.confirm(`Delete \u201c${l.name}\u201d?\n\nThis list will be deleted from every device. The contacts in it stay in Contacts.`)) return;
    if (list === l.guid) setList(ALL);
    void changeList((ls) => ls.filter((x) => x.guid !== l.guid), { guid: l.guid, delete: true });
  };
  const addToList = (l: ContactList, people: string[]) => {
    const fresh = people.filter((p) => !l.members.some((m) => m.toUpperCase() === p.toUpperCase()));
    if (fresh.length === 0) return;
    void changeList(
      (ls) => ls.map((x) => (x.guid === l.guid ? { ...x, members: [...x.members, ...fresh] } : x)),
      { guid: l.guid, add: fresh }
    );
  };
  const removeFromList = (l: ContactList, people: string[]) => {
    const gone = new Set(people.map((p) => p.toUpperCase()));
    void changeList(
      (ls) => ls.map((x) => (x.guid === l.guid ? { ...x, members: x.members.filter((m) => !gone.has(m.toUpperCase())) } : x)),
      { guid: l.guid, remove: people }
    );
  };
  const openList = lists.find((l) => l.guid === list) ?? null;

  /** What a contact's menu offers — the same whether opened by right-click or the card. */
  const contactEntries = (c: Contact, choosingList: boolean): MenuEntry[] => {
    if (choosingList) {
      const into = lists.filter((l) => !l.guid.startsWith("pending-") && !l.members.some((m) => m.toUpperCase() === c.guid.toUpperCase()));
      return [
        ...into.map((l) => ({ label: l.name, run: () => addToList(l, [c.guid]) })),
        ...(into.length ? [null] : []),
        { label: "New List\u2026", run: () => void createList("Untitled List", [c.guid]) },
      ];
    }
    return [
      { label: "Add to List\u2026", keep: true, run: () => setContactMenu((m) => (m ? { ...m, lists: true } : m)) },
      ...(openList ? [{ label: `Remove from \u201c${openList.name}\u201d`, run: () => removeFromList(openList, [c.guid]) }] : []),
      null,
      { label: "Edit Contact", run: () => { setOpenGuid(c.guid); setDraft({ ...c }); } },
      ...(me === c.guid ? [] : [{ label: "Make This My Card", run: () => void makeMine(c) }]),
      { label: "Export vCard\u2026", run: () => void exportCards([c]) },
      { label: "Delete Contact\u2026", danger: true, run: () => {
        askDelete(c);
      } },
    ];
  };

  /** Every write goes to Apple first, then the library is re-read. */
  const write = useCallback(
    async (run: () => Promise<void>) => {
      setBusy(true);
      setError(null);
      try {
        await run();
        // A deleted contact leaves every list it was in, so the lists are read again too.
        await Promise.all([read(search), readLists()]);
        return true;
      } catch (e) {
        setError(messageOf(e));
        return false;
      } finally {
        setBusy(false);
      }
    },
    [read, readLists, search]
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

  /**
   * Deleting a person is not something to discover by accident. Inside a list, icloud.com
   * asks whether the card goes or only its place in the list — the same here.
   */
  const askDelete = (c: Contact) => {
    if (openList) setDeleting(c);
    else if (window.confirm(`Delete ${displayName(c)}?`)) void removeContact(c.guid);
  };

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
  // Who is still here of those picked — a search or a list change narrows them.
  const pickedPeople = useMemo(() => sortedContacts(shown.filter((c) => picked.includes(c.guid))), [shown, picked]);
  const multi = pickedPeople.length > 1;
  const removeMany = async (people: Contact[]) => {
    if (!window.confirm(`Delete ${people.length} contacts?\n\nThey will be deleted from Contacts on every device.`)) return;
    setPicked([]);
    await write(async () => {
      for (const c of people) await api.contactDelete(c.guid);
    });
  };

  return (
    // A contact being written is always what shows; it has its own Cancel.
    <div className={`contacts${detail || draft ? " is-detail" : ""}`}>
      <aside className="contacts-lists" aria-label="Contact lists">
        <p className="notes-side-heading">iCloud</p>
        <ListRow
          name="All Contacts"
          count={contacts.length}
          current={list === ALL}
          everyone
          onOpen={() => setList(ALL)}
        />
        {lists.map((l) =>
          naming === l.guid ? (
            <NameField key={l.guid} initial={l.name} label={`Rename ${l.name}`} onDone={(name) => renameList(l, name)} onCancel={() => setNaming(null)} />
          ) : (
            <ListRow
              key={l.guid}
              name={l.name}
              count={l.members.filter((m) => contacts.some((c) => c.guid.toUpperCase() === m.toUpperCase())).length}
              current={list === l.guid}
              menuOpen={listMenu?.list.guid === l.guid}
              dropping={dropOn === l.guid}
              onOpen={() => setList(l.guid)}
              onMenu={l.guid.startsWith("pending-") ? undefined : (at) => setListMenu({ ...at, list: l })}
              onDragOver={(e) => {
                if (!dragging || l.guid.startsWith("pending-")) return;
                e.preventDefault();
                e.dataTransfer.dropEffect = "copy";
                if (dropOn !== l.guid) setDropOn(l.guid);
              }}
              onDragLeave={() => setDropOn((d) => (d === l.guid ? null : d))}
              onDrop={(e) => {
                e.preventDefault();
                const people = dragging;
                setDragging(null);
                setDropOn(null);
                if (people) addToList(l, people);
              }}
            />
          )
        )}
        {naming === "new" && (
          <NameField initial="Untitled List" label="New list name" onDone={(name) => { setNaming(null); void createList(name); }} onCancel={() => setNaming(null)} />
        )}
        <button className="notes-new-folder" onClick={() => setNaming("new")}>
          <Plus size={15} strokeWidth={2} aria-hidden="true" />
          New List
        </button>
        {listMenu && (
          <RowMenu
            at={listMenu}
            onClose={() => setListMenu(null)}
            entries={[
              { label: "Rename List", run: () => setNaming(listMenu.list.guid) },
              null,
              { label: "Delete List\u2026", danger: true, run: () => deleteList(listMenu.list) },
            ]}
          />
        )}
      </aside>

      <section className="contacts-list" aria-label="Contacts">
        <header className="cal-bar">
          <div className="cal-nav">
            <h1>{openList?.name ?? "Contacts"}</h1>
            <span className="badge">{shown.length}</span>
          </div>
          <div className="cal-actions">
            <button className="quiet" disabled={busy} onClick={() => setDraft(blank())}>
              New contact
            </button>
            <button
              className="quiet"
              aria-label="More"
              aria-haspopup="menu"
              title="More"
              disabled={busy}
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                setMoreMenu({ x: r.left, y: r.bottom + 4, alignRight: false });
              }}
            >
              •••
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

        {/* Shown only when the lists column has folded away. */}
        <div className="contacts-list-picker">
          <Picker
            label="List"
            value={list}
            onChange={setList}
            options={[{ value: ALL, label: "All Contacts" }, ...lists.map((l) => ({ value: l.guid, label: l.name }))]}
          />
        </div>

        {error && <Notice message={error} onDismiss={() => setError(null)} />}
        {done && <p className="drive-saved" role="status">{done}</p>}
        {moreMenu && (
          <RowMenu
            at={moreMenu}
            onClose={() => setMoreMenu(null)}
            entries={[
              { label: "Import vCard\u2026", run: () => void importCards() },
              { label: openList ? `Export \u201c${openList.name}\u201d\u2026` : "Export All Contacts\u2026", run: () => void exportCards(sortedContacts(shown), openList?.name) },
            ]}
          />
        )}

        <ul
          className="contact-rows"
          onKeyDown={(e) => {
            // Command/Ctrl-A: everyone shown.
            if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a") {
              e.preventDefault();
              const everyone = shown.map((c) => c.guid);
              setPicked(everyone.length > 1 ? everyone : []);
            } else if (e.key === "Escape" && picked.length) {
              setPicked([]);
            }
          }}
        >
          {sortedContacts(shown).map((contact, i, all) => (
            <Fragment key={contact.guid}>
            {(i === 0 || letterOf(all[i - 1]) !== letterOf(contact)) && (
              <li className="contact-letter" aria-hidden="true">{letterOf(contact)}</li>
            )}
            <li
              // Dragged onto a list, a person joins it — as in Contacts on the Mac.
              draggable={lists.length > 0}
              onDragStart={(e) => {
                e.dataTransfer.effectAllowed = "copy";
                e.dataTransfer.setData("text/plain", displayName(contact));
                setDragging(picked.includes(contact.guid) ? picked : [contact.guid]);
              }}
              onDragEnd={() => { setDragging(null); setDropOn(null); }}
              onContextMenu={(e) => {
                e.preventDefault();
                setOpenGuid(contact.guid);
                setContactMenu({ x: e.clientX, y: e.clientY, alignRight: false, contact, lists: false });
              }}
            >
              <button
                className="contact-row"
                aria-current={multi ? picked.includes(contact.guid) : openGuid === contact.guid}
                onClick={(e) => {
                  if (e.metaKey || e.ctrlKey) {
                    // Command/Ctrl-click adds or takes away one, keeping the rest.
                    const base = picked.length ? picked : openGuid ? [openGuid] : [];
                    const next = base.includes(contact.guid) ? base.filter((g) => g !== contact.guid) : [...base, contact.guid];
                    setPicked(next.length > 1 ? next : []);
                    if (next.length === 1) setOpenGuid(next[0]);
                    return;
                  }
                  if (e.shiftKey && openGuid) {
                    // Shift-click takes everyone between the last picked and this one.
                    const order = all.map((c) => c.guid);
                    const [a, z] = [order.indexOf(openGuid), order.indexOf(contact.guid)].sort((x, y) => x - y);
                    if (a >= 0) {
                      const range = order.slice(a, z + 1);
                      setPicked(range.length > 1 ? range : []);
                      return;
                    }
                  }
                  setPicked([]);
                  setOpenGuid(contact.guid);
                  setDetail(true);
                }}
              >
                <Avatar contact={contact} />
                <span className="contact-row-name">{displayName(contact)}</span>
                {me === contact.guid && <span className="contact-me" title="My Card">me</span>}
              </button>
            </li>
            </Fragment>
          ))}
        </ul>

        {loaded && shown.length === 0 && !error && (
          <p className="cal-empty">{search ? "No matches." : openList ? "No one in this list yet." : "No contacts yet."}</p>
        )}
        {deleting && openList && (
          <Portal>
            <div className="modal-veil" onClick={() => setDeleting(null)}>
              <div className="modal-sheet cx-ask" role="alertdialog" aria-label="Delete contact" onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => { if (e.key === "Escape") setDeleting(null); }}>
                <h2>Delete {displayName(deleting)}?</h2>
                <p>Remove this contact from &ldquo;{openList.name}&rdquo;, or delete the card from Contacts on every device.</p>
                <div className="cx-ask-actions">
                  <button className="primary" autoFocus onClick={() => { const c = deleting; setDeleting(null); removeFromList(openList, [c.guid]); }}>
                    Remove from List
                  </button>
                  <button className="quiet danger" onClick={() => { const c = deleting; setDeleting(null); void removeContact(c.guid); }}>
                    Delete Card
                  </button>
                  <button className="quiet" onClick={() => setDeleting(null)}>Cancel</button>
                </div>
              </div>
            </div>
          </Portal>
        )}
        {contactMenu && (
          <RowMenu
            at={contactMenu}
            onClose={() => setContactMenu(null)}
            entries={contactEntries(contactMenu.contact, contactMenu.lists)}
          />
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

            <div className="contact-birthday-field">
              <label>
                <span>Birthday</span>
                <input
                  type="date"
                  aria-label="Birthday"
                  value={draft.birthday ? (draft.birthday.startsWith(NO_YEAR) ? `2000${draft.birthday.slice(4, 10)}` : draft.birthday.slice(0, 10)) : ""}
                  onChange={(e) => {
                    const v = e.target.value;
                    if (!v) return setDraft({ ...draft, birthday: null });
                    const unknown = draft.birthday?.startsWith(NO_YEAR) ?? false;
                    setDraft({ ...draft, birthday: unknown ? `${NO_YEAR}${v.slice(4)}` : v });
                  }}
                />
              </label>
              {draft.birthday && (
                <label className="contact-birthday-noyear">
                  <input
                    type="checkbox"
                    checked={draft.birthday.startsWith(NO_YEAR)}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        birthday: e.target.checked
                          ? `${NO_YEAR}${draft.birthday!.slice(4, 10)}`
                          : `${new Date().getFullYear()}${draft.birthday!.slice(4, 10)}`,
                      })
                    }
                  />
                  Year unknown
                </label>
              )}
            </div>

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
        ) : multi ? (
          <div className="contact-many">
            <div className="contact-many-faces" aria-hidden="true">
              {pickedPeople.slice(0, 3).map((c) => <Avatar key={c.guid} contact={c} large />)}
            </div>
            <h2>{pickedPeople.length} Contacts Selected</h2>
            <div className="cal-form-actions">
              <button
                className="quiet"
                aria-haspopup="menu"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  setPickMenu({ x: r.left, y: r.bottom + 4, alignRight: false });
                }}
              >
                Add to List
              </button>
              {openList && (
                <button className="quiet" onClick={() => { removeFromList(openList, pickedPeople.map((c) => c.guid)); setPicked([]); }}>
                  Remove from List
                </button>
              )}
              <button className="quiet" onClick={() => void exportCards(pickedPeople)}>Export vCards</button>
              <button className="quiet danger" disabled={busy} onClick={() => void removeMany(pickedPeople)}>Delete</button>
            </div>
            {pickMenu && (
              <RowMenu
                at={pickMenu}
                onClose={() => setPickMenu(null)}
                entries={[
                  ...lists.filter((l) => !l.guid.startsWith("pending-")).map((l) => ({ label: l.name, run: () => addToList(l, pickedPeople.map((c) => c.guid)) })),
                  ...(lists.length ? [null] : []),
                  { label: "New List\u2026", run: () => void createList("Untitled List", pickedPeople.map((c) => c.guid)) },
                ]}
              />
            )}
          </div>
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
                {me === open.guid && <p className="contact-company">My Card</p>}
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
              {open.birthday && (
                <div>
                  <dt>birthday</dt>
                  <dd>{birthdayText(open.birthday)}</dd>
                </div>
              )}
            </dl>

            {open.methods.length === 0 && !open.birthday && (
              <p className="cal-empty">No phone numbers or addresses saved.</p>
            )}
            {open.notes && <p className="contact-notes">{open.notes}</p>}

            <div className="cal-form-actions">
              <button className="quiet" disabled={busy} onClick={() => setDraft({ ...open })}>
                Edit
              </button>
              <button
                className="quiet"
                aria-haspopup="menu"
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  setContactMenu({ x: r.left, y: r.bottom + 4, alignRight: false, contact: open, lists: true });
                }}
              >
                Add to List
              </button>
              {openList && (
                <button className="quiet" onClick={() => removeFromList(openList, [open.guid])}>
                  Remove from List
                </button>
              )}
              <button
                className="quiet danger"
                disabled={busy}
                onClick={() => askDelete(open)}
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

/** A row in the lists column: All Contacts, or one list, with its count and ⋯. */
function ListRow({
  name,
  count,
  current,
  everyone = false,
  menuOpen = false,
  dropping = false,
  onOpen,
  onMenu,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  name: string;
  count: number;
  current: boolean;
  everyone?: boolean;
  menuOpen?: boolean;
  dropping?: boolean;
  onOpen: () => void;
  onMenu?: (at: MenuAt) => void;
  onDragOver?: React.DragEventHandler;
  onDragLeave?: React.DragEventHandler;
  onDrop?: React.DragEventHandler;
}) {
  const Glyph = everyone ? UsersRound : UserRound;
  return (
    <div
      className={`notes-folder-row${menuOpen ? " is-menu" : ""}${dropping ? " is-drop" : ""}`}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu?.({ x: e.clientX, y: e.clientY, alignRight: false });
      }}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <button className="notes-folder" aria-current={current} onClick={onOpen}>
        <Glyph className="contacts-list-glyph" size={16} strokeWidth={2} aria-hidden="true" />
        <span className="notes-folder-name">{name}</span>
        <span className="notes-folder-count">{count || ""}</span>
      </button>
      {onMenu && (
        <button
          className="notes-folder-more"
          aria-label={`Actions for ${name}`}
          aria-haspopup="menu"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            onMenu({ x: r.right, y: r.bottom + 4, alignRight: true });
          }}
        >
          <span aria-hidden="true">•••</span>
        </button>
      )}
    </div>
  );
}

/** A list's name being typed, in place: Return keeps it, Escape leaves it as it was. */
function NameField({ initial, label, onDone, onCancel }: {
  initial: string;
  label: string;
  onDone: (name: string) => void;
  onCancel: () => void;
}) {
  const settled = useRef(false);
  return (
    <input
      className="notes-folder notes-folder-rename"
      defaultValue={initial}
      aria-label={label}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onBlur={(e) => {
        if (settled.current) return;
        settled.current = true;
        onDone(e.currentTarget.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          settled.current = true;
          onCancel();
        }
      }}
    />
  );
}

/** The year Apple writes for a birthday given without one. */
const NO_YEAR = "1604";

/** "17 March", or "17 March 1998" when the year is known — as a contact card shows it. */
function birthdayText(raw: string): string {
  const [y, m, d] = raw.slice(0, 10).split("-").map(Number);
  if (!m || !d) return raw;
  const date = new Date(y === Number(NO_YEAR) ? 2000 : y, m - 1, d);
  return date.toLocaleDateString(undefined, y === Number(NO_YEAR) ? { day: "numeric", month: "long" } : { dateStyle: "long" });
}

/**
 * The order Contacts keeps: by last name, then first — a company by its name — letters
 * before anything else, accents and case aside, and a card with no name at all ("#") last.
 */
const contactCollator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });
function sortKey(c: Contact): string {
  if (c.isCompany && c.company) return c.company;
  return [c.lastName, c.firstName, c.middleName].filter(Boolean).join(" ") || c.company || "";
}
function letterOf(c: Contact): string {
  const first = sortKey(c).trim().charAt(0).toLocaleUpperCase();
  return /\p{L}/u.test(first) ? first.normalize("NFD").charAt(0) : "#";
}
function sortedContacts(list: Contact[]): Contact[] {
  return [...list].sort((a, b) => {
    const la = letterOf(a) === "#", lb = letterOf(b) === "#";
    if (la !== lb) return la ? 1 : -1;
    return contactCollator.compare(sortKey(a), sortKey(b));
  });
}
