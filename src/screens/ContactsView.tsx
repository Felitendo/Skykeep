import { Fragment, memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Face } from "../components/Face";
import { Unsent, Waiting } from "../components/Unsent";
import { useOutbox } from "../lib/outbox";
import { toast, toastSaved, useErrorToast } from "../components/Toast";
import { ConfirmSheet, useConfirm } from "../components/Confirm";
import { EmptyState } from "../components/EmptyState";
import { SearchField } from "../components/SearchField";
import { PaneTitle } from "../components/PaneTitle";
import { ChevronLeft, Plus, UserPlus, UsersRound, UserRound } from "lucide-react";
import {
  api,
  contactsApi,
  displayName,
  messageOf,
  photoSrc,
  type Contact,
  type ContactList,
  type ContactMethod,
  type AddressEdit,
  type AddressPart,
  type PostalAddress,
} from "../lib/ipc";
import { Arriving } from "../components/EmptyState";
import { useArriving, useSync } from "../lib/sync";
import { useActiveRef, useScreenActive } from "../lib/screen";
import { Picker } from "../components/Picker";
import { RowMenu, type MenuEntry } from "../components/RowMenu";
import { pickOpen } from "../lib/pick";
import { dateText } from "../lib/when";
import { useReveal } from "../lib/reveal";

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
 * Apple's own labels for each kind of detail, as icloud.com's contact editor offers them
 * (contacts2 bundle). Stored in Apple's spelling, so a label picked here reads as the phone's
 * own "mobile" or "work" there, not as a custom label that happens to say the same thing.
 */
const LABELS: Record<string, string[]> = {
  phone: ["MOBILE", "IPHONE", "APPLE WATCH", "HOME", "WORK", "SCHOOL", "MAIN", "HOME FAX", "WORK FAX", "PAGER"],
  email: ["HOME", "WORK", "SCHOOL", "ICLOUD"],
  url: ["HOMEPAGE", "HOME", "WORK", "SCHOOL"],
};
const SHOWN: Record<string, string> = { IPHONE: "iPhone", "APPLE WATCH": "Apple Watch", ICLOUD: "iCloud" };
/** A label as Contacts writes it: Apple's names in their own case, anything else as typed. */
const labelText = (label: string) => SHOWN[label] ?? (label === label.toUpperCase() ? label.toLowerCase() : label);
const CUSTOM = "\u0000custom";

/**
 * A contact's picture where there is one, initials where there is not.
 *
 * The image is allowed to fail — a photo Apple has moved, or no network yet — and when it
 * does the initials underneath are what shows, rather than a broken frame.
 */
function Avatar({ contact, large }: { contact: Contact; large?: boolean }) {
  return (
    <span className={`avatar${large ? " large" : ""}`} aria-hidden="true">
      <Face src={photoSrc(contact)} initials={initials(contact)} />
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
  useErrorToast(error, () => setError(null));
  const [confirm, confirmSheet] = useConfirm();
  const { revision } = useSync();
  // A pass that lands while another screen is in front is read when this one comes back, once,
  // not at every landing: a screen out of sight stays mounted (lib/screen).
  const active = useScreenActive();
  const seen = useRef(revision);
  if (active) seen.current = revision;
  const due = seen.current;
  const arriving = useArriving("contacts");
  /** The card being written, or null while simply looking. */
  const [draft, setDraft] = useState<Contact | null>(null);
  /** Details whose label is being typed rather than picked, by their place on the card. */
  const [customAt, setCustomAt] = useState<Set<number>>(() => new Set());
  /**
   * The card as the form opened it. Saving writes only what differs from it, over Apple's
   * card as it is then — so a change made on the phone while the form was open stays.
   */
  const draftBase = useRef<Contact | null>(null);
  /**
   * The open card's addresses part by part: as last read, or as just edited here — drawn at
   * once, and put back if Apple turns the change down.
   */
  const [addresses, setAddresses] = useState<{ guid: string; list: PostalAddress[] } | null>(null);
  /** Address writes go to Apple one after another; while any is out, a read does not undo it. */
  const addressQueue = useRef<Promise<unknown>>(Promise.resolve());
  const addressesOut = useRef(0);
  /** The addresses being written, and the list the form opened them from. */
  const [draftAddresses, setDraftAddresses] = useState<DraftAddress[]>([]);
  const addressBase = useRef<PostalAddress[]>([]);
  /** Addresses whose label is being typed, by their key. */
  const [addressCustom, setAddressCustom] = useState<Set<number>>(() => new Set());
  const addressKey = useRef(0);
  /** A just-added address takes the focus, once. */
  const [addedKey, setAddedKey] = useState<number | null>(null);
  const startAddresses = (list: PostalAddress[]) => {
    addressBase.current = list;
    setDraftAddresses(list.map((address, from) => ({ key: ++addressKey.current, from, address })));
    setAddressCustom(new Set());
    setAddedKey(null);
  };
  const edit = (c: Contact) => {
    draftBase.current = c;
    setDraft({ ...c });
    setCustomAt(new Set());
    if (addresses?.guid === c.guid) startAddresses(addresses.list);
    else {
      startAddresses([]);
      contactsApi
        .addresses(c.guid)
        .then((list) => {
          if (draftBase.current === c) startAddresses(Array.isArray(list) ? list : []);
        })
        .catch(() => {});
    }
  };
  const [busy, setBusy] = useState(false);
  /** The detail whose label was just clicked to copy it, said for a moment. */
  const [copied, setCopied] = useState<string | null>(null);
  useEffect(() => {
    if (!copied) return undefined;
    const timer = window.setTimeout(() => setCopied(null), 1200);
    return () => window.clearTimeout(timer);
  }, [copied]);
  const [lists, setLists] = useState<ContactList[]>([]);
  /** The list being looked at, or ALL. */
  const [list, setList] = useState<string>(ALL);
  // A contact found by the search across everything: shown, from All Contacts and with this
  // screen's own search cleared, so nothing here hides it.
  useReveal("contacts", (r) => {
    setList(ALL);
    setSearch("");
    setDraft(null);
    setOpenGuid(r.id);
    setDetail(true);
  });
  /** A list whose name is being typed: "new" for one not made yet. */
  const [naming, setNaming] = useState<string | null>(null);
  const [listMenu, setListMenu] = useState<(MenuAt & { list: ContactList }) | null>(null);
  /** A contact's menu: its actions, or — one step in — the lists it can go into. */
  const [contactMenu, setContactMenu] = useState<(MenuAt & { contact: Contact; lists: boolean }) | null>(null);
  /** The card that is this person's own. */
  const [me, setMe] = useState<string | null>(null);
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

  /** Cards saved here and not yet at Apple, and any Apple turned down. */
  const outbox = useOutbox("contacts");
  /** Each read is numbered: one overtaken by a later one (a key typed since) never draws. */
  const reads = useRef(0);
  const read = useCallback(async (query: string) => {
    const mine = ++reads.current;
    const found = await api.contactsList(query.trim() || null);
    if (mine !== reads.current) return;
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
    // Re-reading on every keystroke is the point: the filter runs in SQLite. `outbox.tick`: a
    // card sent, or turned down, changes what the library holds.
  }, [read, search, due, outbox.tick]);

  const readLists = useCallback(async () => {
    const [found, mine] = await Promise.all([api.contactLists(), api.contactMe()]);
    setLists(Array.isArray(found) ? found : []);
    setMe(typeof mine === "string" ? mine : null);
  }, []);

  /** People out as a vCard, named as icloud.com names the file. */
  const exportCards = async (people: Contact[], label?: string) => {
    if (people.length === 0) return;
    const name =
      people.length === 1 ? displayName(people[0])
      : people.length === 2 ? `${displayName(people[0])} and ${displayName(people[1])}`
      : label ?? "iCloud vCards";
    try {
      const path = await api.contactsExport(people.map((c) => c.guid), `${name}.vcf`);
      toastSaved(path);
    } catch (e) {
      setError(messageOf(e));
    }
  };
  const importCards = async () => {
    try {
      // Any number of files at once, vCard or a CSV exported from Google Contacts or Outlook.
      const paths =
        (await pickOpen({
          multiple: true,
          title: "Import Contacts",
          filters: [{ name: "Contacts", extensions: ["vcf", "vcard", "csv"] }],
        })) ?? [];
      if (paths.length === 0) return;
      setBusy(true);
      const [added, failed, known] = await api.contactsImport(paths);
      await Promise.all([read(search), readLists()]);
      const people = (n: number) => `${n} ${n === 1 ? "contact" : "contacts"}`;
      toast(
        [
          `${people(added)} imported`,
          known ? `${people(known)} already in Contacts ${known === 1 ? "was" : "were"} left out` : "",
          failed ? `${failed} could not be read` : "",
        ]
          .filter(Boolean)
          .join(", ") + ".",
      );
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
  }, [readLists, due]);
  // A list deleted elsewhere is not left open, empty, with nothing to say why.
  useEffect(() => {
    if (list !== ALL && !lists.some((l) => l.guid === list)) setList(ALL);
  }, [lists, list]);

  // The open card's addresses, part by part — read again whenever the library may have moved,
  // unless a change of this screen's own is still on its way to Apple.
  useEffect(() => {
    if (!openGuid || openGuid.startsWith("pending-")) return undefined;
    let cancelled = false;
    if (addressesOut.current === 0)
      contactsApi
        .addresses(openGuid)
        .then((list) => {
          if (!cancelled && addressesOut.current === 0) setAddresses({ guid: openGuid, list: Array.isArray(list) ? list : [] });
        })
        .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [openGuid, due, outbox.tick]);

  /**
   * Address edits: drawn at once, then sent. Turned down, the list goes back to what it was,
   * then to what Apple holds, and a toast says so with the way to try again.
   */
  const sendAddresses = useCallback(
    (guid: string, base: PostalAddress[], edits: AddressEdit[], after: PostalAddress[]) => {
      setAddresses({ guid, list: after });
      addressesOut.current += 1;
      addressQueue.current = addressQueue.current.then(async () => {
        try {
          const written = await contactsApi.addressEdit(guid, base, edits);
          if (Array.isArray(written)) setAddresses((cur) => (cur?.guid === guid ? { guid, list: written } : cur));
          read(search).catch(() => {});
        } catch (e) {
          setAddresses((cur) => (cur?.guid === guid ? { guid, list: base } : cur));
          toast({
            message: `Address not saved. ${messageOf(e)}`,
            tone: "error",
            action: { label: "Try Again", run: () => sendAddresses(guid, base, edits, after) },
          });
          const held = await contactsApi.addresses(guid).catch(() => null);
          if (Array.isArray(held)) setAddresses((cur) => (cur?.guid === guid ? { guid, list: held } : cur));
        } finally {
          addressesOut.current -= 1;
        }
      });
    },
    [read, search],
  );

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
    void confirm({
      title: `Delete \u201c${l.name}\u201d?`,
      body: "This list will be deleted from every device. The contacts in it stay in Contacts.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    }).then((sure) => {
      if (!sure) return;
      if (list === l.guid) setList(ALL);
      void changeList((ls) => ls.filter((x) => x.guid !== l.guid), { guid: l.guid, delete: true });
    });
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
        { label: "New List", run: () => void createList("Untitled List", [c.guid]) },
      ];
    }
    return [
      // A submenu of the lists, beside the line, as Contacts on the Mac has it.
      { label: "Add to List", submenu: contactEntries(c, true) },
      ...(openList ? [{ label: `Remove from \u201c${openList.name}\u201d`, run: () => removeFromList(openList, [c.guid]) }] : []),
      null,
      { label: "Edit Contact", run: () => { setOpenGuid(c.guid); edit(c); } },
      ...(me === c.guid ? [] : [{ label: "Make This My Card", run: () => void makeMine(c) }]),
      { label: "Export vCard", run: () => void exportCards([c]) },
      { label: "Delete Contact", danger: true, run: () => {
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
    // Addresses are written part by part on their own; the rest of the card only if it changed.
    const base = addressBase.current;
    const edits = saving.guid ? addressEdits(base, draftAddresses) : [];
    const after = predicted(base, draftAddresses);
    const cardChanged = !saving.guid || JSON.stringify(saving) !== JSON.stringify(draftBase.current);
    if (!cardChanged) {
      setDraft(null);
      if (edits.length) sendAddresses(saving.guid, base, edits, after);
      return;
    }
    if (edits.length) setAddresses({ guid: saving.guid, list: after });
    const ok = await write(async () => {
      guid = await api.contactSave(saving, saving.guid ? draftBase.current : null);
    });
    if (ok) {
      setDraft(null);
      setOpenGuid(guid);
      if (edits.length) sendAddresses(guid, base, edits, after);
    } else if (edits.length) setAddresses({ guid: saving.guid, list: base });
  }, [draft, draftAddresses, sendAddresses, write]);

  /**
   * Deleting a person is not something to discover by accident. Inside a list, icloud.com
   * asks whether the card goes or only its place in the list — the same here.
   */
  const askDelete = (c: Contact) => {
    if (openList) setDeleting(c);
    else
      void confirm({ title: `Delete ${displayName(c)}?`, body: "The card is deleted from Contacts on every device.", action: "Delete", danger: true, symbol: "trash" }).then(
        (sure) => sure && void removeContact(c.guid),
      );
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
  /** The open card's addresses part by part, once read; until then the lines Apple formatted. */
  const structured = open && addresses?.guid === open.guid ? addresses.list : null;
  // Who is still here of those picked — a search or a list change narrows them.
  const pickedPeople = useMemo(() => {
    const chosen = new Set(picked);
    return sortedContacts(shown.filter((c) => chosen.has(c.guid)));
  }, [shown, picked]);
  const multi = pickedPeople.length > 1;
  const pickedSet = useMemo(() => new Set(picked), [picked]);
  /** How many of each list's people are here, counted once per change rather than per render. */
  const listCounts = useMemo(() => {
    const here = new Set(contacts.map((c) => c.guid.toUpperCase()));
    return new Map(lists.map((l) => [l.guid, l.members.filter((m) => here.has(m.toUpperCase())).length]));
  }, [contacts, lists]);
  const removeMany = async (people: Contact[]) => {
    const sure = await confirm({
      title: `Delete ${people.length} contacts?`,
      body: "They will be deleted from Contacts on every device.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (!sure) return;
    setPicked([]);
    await write(async () => {
      for (const c of people) await api.contactDelete(c.guid);
    });
  };

  /**
   * Sorted once per change to who is shown, not on every render: typing in the card's form
   * re-sorted the whole address book with each key, and redrew every row with it.
   */
  const sorted = useMemo(() => sortedContacts(shown), [shown]);
  /** Each person's letter, worked out once per list rather than twice per row per render. */
  const lettered = useMemo(() => sorted.map((c, i, all) => {
    const letter = letterOf(c);
    return { contact: c, letter, starts: i === 0 || letterOf(all[i - 1]) !== letter };
  }), [sorted]);
  const rowsRef = useRef<HTMLUListElement | null>(null);
  const [listFocused, setListFocused] = useState(false);
  const searchRef = useRef<HTMLInputElement | null>(null);
  /** The row for a person brought into view and given focus, as the arrows and the index do. */
  const showRow = (guid: string, focus: boolean) =>
    requestAnimationFrame(() => {
      const row = rowsRef.current?.querySelector<HTMLButtonElement>(`.contact-row[data-guid="${CSS.escape(guid)}"]`);
      row?.scrollIntoView({ block: "nearest" });
      if (focus) row?.focus({ preventScroll: true });
    });

  /**
   * What a row does, as one object that never changes, so a row is redrawn only when what it
   * shows changes — with a long address book, a click redrew every row in it. The handlers
   * read the selection as it is at the moment they run.
   */
  const latest = useRef({ picked, openGuid, sorted });
  latest.current = { picked, openGuid, sorted };
  const rowActions = useMemo<RowActions>(
    () => ({
      click: (contact, e) => {
        const { picked, openGuid, sorted } = latest.current;
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
          const order = sorted.map((c) => c.guid);
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
      },
      dragStart: (contact, e) => {
        const { picked } = latest.current;
        e.dataTransfer.effectAllowed = "copy";
        e.dataTransfer.setData("text/plain", displayName(contact));
        setDragging(picked.includes(contact.guid) ? picked : [contact.guid]);
      },
      dragEnd: () => {
        setDragging(null);
        setDropOn(null);
      },
      menu: (contact, e) => {
        e.preventDefault();
        setOpenGuid(contact.guid);
        setContactMenu({ x: e.clientX, y: e.clientY, alignRight: false, contact, lists: false });
      },
    }),
    [],
  );

  /** The letters the shown names begin with, for the index down the list's edge. */
  const letters = useMemo(() => lettered.filter((r) => r.starts).map((r) => r.letter), [lettered]);
  /** The index earns its column only when the list is longer than the window shows. */
  const [overflowing, setOverflowing] = useState(false);
  useEffect(() => {
    const list = rowsRef.current;
    if (!list || typeof ResizeObserver === "undefined") return undefined;
    const measure = () => setOverflowing(list.scrollHeight > list.clientHeight + 1);
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(list);
    return () => watch.disconnect();
  }, [sorted]);
  const jumpTo = (letter: string) => {
    const list = rowsRef.current;
    const head = list?.querySelector<HTMLElement>(`.contact-letter[data-letter="${CSS.escape(letter)}"]`);
    if (list && head) list.scrollTop = head.offsetTop;
  };

  // The keys Contacts on the Mac answers (Ctrl in place of Command here): ⌘N a new card,
  // ⌘F the search, the arrows walk the list — Shift with them widens the selection — Return
  // edits the card, Delete deletes it, and Escape leaves a card being written unsaved.
  const live = useActiveRef();
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    if (!live.current || e.defaultPrevented) return;
    // A menu, a picker's list or a sheet open over the screen has the keys to itself.
    if (document.querySelector('[role="menu"], [role="listbox"], [role="dialog"], [role="alertdialog"]')) return;
    const target = e.target as HTMLElement | null;
    const inSearch = target === searchRef.current;
    const typing = !inSearch && !!target?.closest?.("input, textarea, select, [contenteditable]");
    const command = (e.metaKey || e.ctrlKey) && !e.altKey;
    const k = e.key.toLowerCase();
    if (command && !e.shiftKey && k === "n") {
      e.preventDefault();
      if (!busy) {
        setPicked([]);
        setDraft(blank());
      }
      return;
    }
    if (command && !e.shiftKey && k === "f") {
      e.preventDefault();
      if (draft) return;
      setDetail(false);
      searchRef.current?.focus();
      searchRef.current?.select();
      return;
    }
    if (draft) {
      // Escape, from a field or not, leaves the card being written, as Cancel does.
      if (e.key === "Escape") {
        e.preventDefault();
        setDraft(null);
      }
      return;
    }
    if (typing || e.altKey || command) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (sorted.length === 0) return;
      e.preventDefault();
      const order = sorted.map((c) => c.guid);
      const at = openGuid ? order.indexOf(openGuid) : -1;
      const nextAt = Math.min(order.length - 1, Math.max(0, at + (e.key === "ArrowDown" ? 1 : -1)));
      const next = order[at < 0 ? 0 : nextAt];
      if (e.shiftKey && openGuid && at >= 0) {
        // Shift and an arrow: the run from where it began to here, as in Finder.
        const base = picked.length ? picked : [openGuid];
        const anchor = base[0];
        const [a, z] = [order.indexOf(anchor), order.indexOf(next)].sort((x, y) => x - y);
        const range = order.slice(a, z + 1);
        setPicked(range.length > 1 ? (anchor === range[0] ? range : [anchor, ...range.filter((g) => g !== anchor)]) : []);
        setOpenGuid(next);
      } else {
        setPicked([]);
        setOpenGuid(next);
      }
      showRow(next, !inSearch);
      return;
    }
    if (inSearch) {
      // Return in the search field goes to the first match, as Contacts does.
      if (e.key === "Enter" && sorted.length) {
        e.preventDefault();
        setOpenGuid(sorted[0].guid);
        setDetail(true);
        showRow(sorted[0].guid, true);
      }
      return;
    }
    if (e.key === "Enter" && open && !multi && !busy) {
      // A button with the focus would press itself; Return on the list means "edit".
      if (target?.closest?.("button") && !target.closest(".contact-row")) return;
      e.preventDefault();
      edit(open);
    } else if ((e.key === "Delete" || e.key === "Backspace") && !busy) {
      if (target?.closest?.(".contact-detail button")) return;
      e.preventDefault();
      if (multi) void removeMany(pickedPeople);
      else if (open) askDelete(open);
    }
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    // A contact being written is always what shows; it has its own Cancel.
    <div className={`contacts${detail || draft ? " is-detail" : ""}`}>
      <aside className="contacts-lists" aria-label="Contact lists">
        <p className="side-heading">iCloud</p>
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
              count={listCounts.get(l.guid) ?? 0}
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
        <button className="side-add" onClick={() => setNaming("new")}>
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
              { label: "Delete List", danger: true, run: () => deleteList(listMenu.list) },
            ]}
          />
        )}
      </aside>

      <section className="contacts-list" aria-label="Contacts">
        <header className="pane-head contacts-head">
          <PaneTitle count={`${shown.length} ${shown.length === 1 ? "contact" : "contacts"}`}>{openList?.name ?? "Contacts"}</PaneTitle>
          <div className="contacts-head-actions">
            <button className="icon-button" aria-label="New contact" title="New Contact" disabled={busy} onClick={() => setDraft(blank())}>
              <UserPlus size={18} strokeWidth={2} aria-hidden="true" />
            </button>
            <button
              className="icon-button"
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

        <SearchField
          ref={searchRef}
          className="contacts-search"
          value={search}
          onChange={setSearch}
          placeholder="Search"
          aria-label="Search contacts"
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

        <Unsent refused={outbox.refused} what="a change to a contact" onResend={(r) => void outbox.resend(r)} onDiscard={(r) => void outbox.discard(r)} />
        {moreMenu && (
          <RowMenu
            at={moreMenu}
            onClose={() => setMoreMenu(null)}
            entries={[
              { label: "Import Contacts", run: () => void importCards() },
              { label: openList ? `Export \u201c${openList.name}\u201d` : "Export All Contacts", run: () => void exportCards(sorted, openList?.name) },
            ]}
          />
        )}

        <div className="contact-rows-wrap">
        <ul
          className={`contact-rows${listFocused ? " is-focused" : ""}`}
          ref={rowsRef}
          // Whether the focus is in the list, for the selection's colour — a class rather than
          // :focus-within, which made WebKit restyle every row at each arrow press.
          onFocus={() => setListFocused(true)}
          onBlur={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setListFocused(false);
          }}
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
          {lettered.map(({ contact, letter, starts }) => (
            <Fragment key={contact.guid}>
            {starts && (
              <li className="contact-letter" data-letter={letter} aria-hidden="true">{letter}</li>
            )}
            <ContactRow
              contact={contact}
              current={multi ? pickedSet.has(contact.guid) : openGuid === contact.guid}
              mine={me === contact.guid}
              draggable={lists.length > 0}
              waiting={outbox.waiting.has(contact.guid)}
              actions={rowActions}
            />
            </Fragment>
          ))}
        </ul>
        {/* The letters down the edge, as icloud.com and the phone index a long list: a press,
            or a drag along them, goes straight to that letter. */}
        {letters.length > 1 && overflowing && (
          <nav
            className="contact-index"
            aria-label="Jump to letter"
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              const letter = (e.target as HTMLElement).closest?.<HTMLElement>(".contact-index button")?.dataset.letter;
              if (letter) jumpTo(letter);
              e.currentTarget.setPointerCapture?.(e.pointerId);
            }}
            onPointerMove={(e) => {
              if (!e.currentTarget.hasPointerCapture?.(e.pointerId)) return;
              const under = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
              const letter = under?.closest?.<HTMLElement>(".contact-index button")?.dataset.letter;
              if (letter) jumpTo(letter);
            }}
          >
            {letters.map((l) => (
              <button key={l} type="button" tabIndex={-1} data-letter={l} aria-label={`Contacts under ${l}`} onClick={() => jumpTo(l)}>
                {l}
              </button>
            ))}
          </nav>
        )}
        </div>

        {shown.length === 0 && !search && <Arriving service="contacts" what="contacts" />}
        {loaded && shown.length === 0 && !error && !(arriving && !search) && (
          <EmptyState className="contact-empty">{search ? "No matches." : openList ? "No one in this list yet." : "No contacts yet."}</EmptyState>
        )}
        {deleting && openList && (
          <ConfirmSheet<"list" | "card">
            title={`Delete ${displayName(deleting)}?`}
            label="Delete contact"
            body={<>Remove this contact from &ldquo;{openList.name}&rdquo;, or delete the card from Contacts on every device.</>}
            choices={[
              { label: "Remove from List", value: "list", primary: true, focus: true },
              { label: "Delete Card", value: "card", danger: true },
            ]}
            onAnswer={(how) => {
              const c = deleting;
              setDeleting(null);
              if (how === "list") removeFromList(openList, [c.guid]);
              else void removeContact(c.guid);
            }}
            onCancel={() => setDeleting(null)}
          />
        )}
        {confirmSheet}
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
                // An existing card's addresses are written part by part, below.
                m.kind === "address" && draft.guid ? null : !EDITABLE_KINDS.has(m.kind) ? (
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
                  {(() => {
                    const standard = LABELS[m.kind] ?? [];
                    // Apple's names match whatever their case, so "mobile" is the phone's own.
                    const match = standard.find((v) => v === m.label?.toUpperCase());
                    const custom = customAt.has(index) || (m.label !== null && !match);
                    return (
                      <span className="contact-label-cell">
                        <Picker
                          label={`Label for detail ${index + 1}`}
                          className="contact-label-pick"
                          value={custom ? CUSTOM : (match ?? "")}
                          onChange={(v) => {
                            if (v === CUSTOM) {
                              setCustomAt((was) => new Set(was).add(index));
                              return;
                            }
                            setCustomAt((was) => { const next = new Set(was); next.delete(index); return next; });
                            editMethod(index, { label: v || null });
                          }}
                          options={[
                            ...(m.label === null && !custom ? [{ value: "", label: "no label" }] : []),
                            ...standard.map((v) => ({ value: v, label: labelText(v) })),
                            { value: CUSTOM, label: "Custom" },
                          ]}
                        />
                        {custom && (
                          <input
                            className="contact-method-label"
                            value={m.label ?? ""}
                            placeholder="label"
                            aria-label={`Custom label for detail ${index + 1}`}
                            onChange={(e) => editMethod(index, { label: e.target.value || null })}
                          />
                        )}
                      </span>
                    );
                  })()}
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
                    onClick={() => {
                      setDraft({
                        ...draft,
                        methods: draft.methods.filter((_, i) => i !== index),
                      });
                      // Places shift up by one; which were being typed is worked out again.
                      setCustomAt(new Set());
                    }}
                  >
                    ✕
                  </button>
                </li>
                ),
              )}
            </ul>

            {draft.guid && draftAddresses.length > 0 && (
              <ul className="contact-method-rows contact-address-rows" aria-label="Addresses">
                {draftAddresses.map((d, i) => (
                  <AddressEditor
                    key={d.key}
                    n={i + 1}
                    address={d.address}
                    custom={addressCustom.has(d.key)}
                    autoFocus={addedKey === d.key}
                    onChange={(patch) =>
                      setDraftAddresses((all) => all.map((x) => (x.key === d.key ? { ...x, address: { ...x.address, ...patch } } : x)))
                    }
                    onCustom={(on) =>
                      setAddressCustom((was) => {
                        const next = new Set(was);
                        if (on) next.add(d.key);
                        else next.delete(d.key);
                        return next;
                      })
                    }
                    onRemove={() => setDraftAddresses((all) => all.filter((x) => x.key !== d.key))}
                  />
                ))}
              </ul>
            )}

            <div className="contact-form-adds">
              <button
                type="button"
                className="quiet"
                onClick={() =>
                  setDraft({
                    ...draft,
                    methods: [...draft.methods, { kind: "phone", label: "MOBILE", value: "" }],
                  })
                }
              >
                Add phone, email or link
              </button>
              {draft.guid && (
                <button
                  type="button"
                  className="quiet"
                  onClick={() => {
                    const key = ++addressKey.current;
                    setDraftAddresses((all) => [...all, { key, from: null, address: blankAddress() }]);
                    setAddedKey(key);
                  }}
                >
                  Add address
                </button>
              )}
            </div>

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
                  { label: "New List", run: () => void createList("Untitled List", pickedPeople.map((c) => c.guid)) },
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
              {(structured ? open.methods.filter((m) => m.kind !== "address") : open.methods).map((m, i) => {
                const key = `${m.kind}-${i}`;
                const label = (m.label && labelText(m.label)) || KIND_LABEL[m.kind] || m.kind;
                return (
                <div key={key}>
                  {/* The label copies what is beside it, as a click on one does in Contacts. */}
                  <dt>
                    <button
                      type="button"
                      className="contact-copy"
                      title={`Copy ${label}`}
                      aria-label={`Copy ${label}`}
                      onClick={() => void copyText(m.value).then((ok) => ok && setCopied(key))}
                    >
                      <span className={copied === key ? "is-hidden" : undefined}>{label}</span>
                      <span className={copied === key ? undefined : "is-hidden"} aria-hidden={copied !== key}>copied</span>
                    </button>
                  </dt>
                  <dd>
                    {m.kind === "email" ? (
                      <a href={`mailto:${m.value}`}>{m.value}</a>
                    ) : m.kind === "phone" ? (
                      <a href={`tel:${m.value.replace(/[^\d+#*,;]/g, "")}`}>{m.value}</a>
                    ) : m.kind === "url" ? (
                      <a href={webHref(m.value)}>{m.value}</a>
                    ) : m.kind === "address" ? (
                      // A postal address is several lines and stays that way.
                      <>
                        <span className="contact-address">{m.value}</span>
                        <a className="contact-maps" href={mapsHref(m.value)}>Show in Maps</a>
                      </>
                    ) : (
                      m.value
                    )}
                  </dd>
                </div>
                );
              })}
              {structured?.map((a, i) => {
                const key = `postal-${i}`;
                const label = (a.label && labelText(a.label)) || "address";
                const text = formatAddress(a);
                return (
                  <div key={key} className="contact-postal">
                    <dt>
                      <button
                        type="button"
                        className="contact-copy"
                        title={`Copy ${label} address`}
                        aria-label={`Copy ${label} address`}
                        onClick={() => void copyText(text).then((ok) => ok && setCopied(key))}
                      >
                        <span className={copied === key ? "is-hidden" : undefined}>{label}</span>
                        <span className={copied === key ? undefined : "is-hidden"} aria-hidden={copied !== key}>copied</span>
                      </button>
                    </dt>
                    <dd>
                      <span className="contact-address">{text}</span>
                      <a className="contact-maps" href={mapsHref(text)}>Show in Maps</a>
                    </dd>
                  </div>
                );
              })}
              {open.birthday && (
                <div>
                  <dt>birthday</dt>
                  <dd>{birthdayText(open.birthday)}</dd>
                </div>
              )}
            </dl>

            {open.methods.length === 0 && !structured?.length && !open.birthday && (
              <p className="contact-none">No phone numbers or addresses saved.</p>
            )}
            {open.notes && <p className="contact-notes">{open.notes}</p>}

            <div className="cal-form-actions">
              <button className="quiet" disabled={busy} onClick={() => edit(open)}>
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
          loaded && <EmptyState className="contact-pick">Select a contact.</EmptyState>
        )}
      </article>
    </div>
  );
}

type RowActions = {
  click: (contact: Contact, e: React.MouseEvent) => void;
  dragStart: (contact: Contact, e: React.DragEvent) => void;
  dragEnd: () => void;
  menu: (contact: Contact, e: React.MouseEvent) => void;
};

/**
 * One person in the list. Drawn again only when what it shows changes — its name, its picture,
 * whether it is selected — so a click or a key in a long address book touches two rows, not
 * every one. The layout of a row off screen is skipped too (content-visibility, in the CSS).
 */
const ContactRow = memo(function ContactRow({ contact, current, mine, draggable, waiting, actions }: {
  contact: Contact;
  current: boolean;
  mine: boolean;
  draggable: boolean;
  /** Saved here, not yet at Apple. */
  waiting: boolean;
  actions: RowActions;
}) {
  return (
    <li
      // Dragged onto a list, a person joins it — as in Contacts on the Mac.
      draggable={draggable}
      onDragStart={(e) => actions.dragStart(contact, e)}
      onDragEnd={actions.dragEnd}
      onContextMenu={(e) => actions.menu(contact, e)}
    >
      <button
        className="contact-row"
        data-guid={contact.guid}
        aria-current={current}
        onClick={(e) => actions.click(contact, e)}
      >
        <Avatar contact={contact} />
        <span className="contact-row-name">{displayName(contact)}</span>
        {mine && <span className="contact-me" title="My Card">me</span>}
        <Waiting shown={waiting} />
      </button>
    </li>
  );
});

/** Text onto the clipboard; the older way where the newer one is refused. */
async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const field = document.createElement("textarea");
    field.value = text;
    field.setAttribute("readonly", "");
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.appendChild(field);
    field.select();
    let copied = false;
    try {
      copied = document.execCommand("copy");
    } catch {
      copied = false;
    }
    field.remove();
    return copied;
  }
}

/** A link a person typed without its scheme still opens: "example.com" is the web. */
const webHref = (value: string) => (/^[a-z][a-z0-9+.-]*:/i.test(value.trim()) ? value.trim() : `https://${value.trim()}`);

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
      <button className="source-row notes-folder" aria-current={current} onClick={onOpen}>
        <Glyph className="contacts-list-glyph" size={16} strokeWidth={2} aria-hidden="true" />
        <span className="source-name notes-folder-name">{name}</span>
        <span className="count notes-folder-count">{count || ""}</span>
      </button>
      {onMenu && (
        <button
          className="icon-button small round row-more notes-folder-more"
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
      className="field notes-folder-rename"
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
  return dateText(date, y === Number(NO_YEAR) ? { day: "numeric", month: "long" } : { dateStyle: "long" });
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

/* ── Postal addresses ─────────────────────────────────────────────────────────────────── */

/**
 * The lines of an address in the order its country writes them — the backend's
 * `PostalAddress::formatted`, line for line, so an address just edited here reads exactly as
 * it will once the card is read back from Apple.
 */
type AddressLine = [AddressPart, string][];
const LAYOUTS: Record<string, AddressLine[]> = {
  us: [[["street", ""]], [["subLocality", ""]], [["city", ""], ["state", ", "], ["postalCode", " "]], [["country", ""]]],
  gb: [[["street", ""]], [["subLocality", ""]], [["city", ""]], [["subAdministrativeArea", ""]], [["postalCode", ""]], [["country", ""]]],
  in: [[["street", ""]], [["subLocality", ""]], [["city", ""], ["postalCode", " "]], [["state", ""]], [["country", ""]]],
  de: [[["street", ""]], [["postalCode", ""], ["city", " "]], [["country", ""]]],
  jp: [[["postalCode", ""]], [["state", ""], ["city", " "], ["subLocality", " "]], [["street", ""]], [["country", ""]]],
  other: [[["street", ""]], [["subLocality", ""]], [["city", ""], ["state", " "], ["postalCode", " "]], [["country", ""]]],
};
LAYOUTS.fr = LAYOUTS.de;
const CODES: Record<string, string> = { us: "us", gb: "gb", uk: "gb", in: "in", de: "de", fr: "fr", jp: "jp" };
const NAMES: Record<string, string> = Object.fromEntries(
  ([
    ["us", ["united states", "united states of america", "usa", "us", "u.s.a", "u.s", "america"]],
    ["gb", ["united kingdom", "uk", "u.k", "great britain", "britain", "england", "scotland", "wales", "northern ireland", "gb"]],
    ["in", ["india", "bharat", "in"]],
    ["de", ["germany", "deutschland", "de"]],
    ["fr", ["france", "fr"]],
    ["jp", ["japan", "nippon", "日本", "jp"]],
  ] as const).flatMap(([code, names]) => names.map((n) => [n, code])),
);
/** Apple's code for a country typed by name, for the countries laid out by their own rules. */
function countryCodeFor(name: string): string | null {
  const folded = name.trim().normalize("NFD").replace(/\p{M}/gu, "").toLowerCase().replace(/\.+$/, "");
  return NAMES[folded] ?? null;
}
function formatAddress(a: PostalAddress): string {
  const key = (a.countryCode && CODES[a.countryCode.trim().toLowerCase()]) || (a.country && countryCodeFor(a.country)) || "other";
  const lines: string[] = [];
  for (const line of LAYOUTS[key]) {
    let text = "";
    for (const [part, separator] of line) {
      const value = a[part];
      if (!value) continue;
      if (text) text += separator;
      text += value;
    }
    lines.push(...text.split("\n").map((l) => l.trim()).filter(Boolean));
  }
  return lines.join("\n");
}
/** The address on one line, for a map to search for. */
const mapsHref = (text: string) => `geo:0,0?q=${encodeURIComponent(text.split("\n").join(", "))}`;

/** The parts the editor shows, in the order Tab walks them. */
const ADDRESS_PARTS = ["street", "city", "state", "postalCode", "country"] as const;
const PART_NAME: Record<(typeof ADDRESS_PARTS)[number], string> = {
  street: "Street",
  city: "City",
  state: "State",
  postalCode: "Postcode",
  country: "Country",
};
const ADDRESS_LABELS = ["HOME", "WORK", "OTHER"];
const ALL_PARTS: AddressPart[] = ["street", "subLocality", "city", "subAdministrativeArea", "state", "postalCode", "country", "countryCode"];

const blankAddress = (): PostalAddress => ({
  label: "HOME", street: null, subLocality: null, city: null, subAdministrativeArea: null,
  state: null, postalCode: null, country: null, countryCode: null,
});
/** A part as stored: trimmed, and nothing rather than an empty string. */
const tidy = (v: string | null | undefined) => (v && v.trim() ? v.trim() : null);
const isEmptyAddress = (a: PostalAddress) => ALL_PARTS.every((p) => !tidy(a[p]));

/** An address in the editor: where it sat in the list the form opened (null for a new one). */
type DraftAddress = { key: number; from: number | null; address: PostalAddress };

/**
 * What the editor's addresses change from the list it opened with, as the backend's edits:
 * parts and labels first (by their places in that list), then removals from the end back,
 * then additions — so every index names the address it meant.
 */
function addressEdits(base: PostalAddress[], draft: DraftAddress[]): AddressEdit[] {
  const edits: AddressEdit[] = [];
  for (const d of draft) {
    if (d.from === null) continue;
    const was = base[d.from];
    for (const part of ADDRESS_PARTS) {
      const value = tidy(d.address[part]);
      if (value !== tidy(was[part])) edits.push({ op: "setField", index: d.from, field: part, value });
    }
    const label = tidy(d.address.label);
    if (label !== tidy(was.label)) edits.push({ op: "setLabel", index: d.from, label });
  }
  const kept = new Set(draft.map((d) => d.from));
  for (let i = base.length - 1; i >= 0; i--) if (!kept.has(i)) edits.push({ op: "remove", index: i });
  for (const d of draft) {
    if (d.from === null && !isEmptyAddress(d.address)) edits.push({ op: "add", address: settled(d.address, null) });
  }
  return edits;
}

/** An address as the backend will store it: trimmed, and its country's code kept in step. */
function settled(a: PostalAddress, was: PostalAddress | null): PostalAddress {
  const out = { ...a, label: tidy(a.label) };
  for (const p of ALL_PARTS) out[p] = tidy(a[p]);
  if (!was ? !out.countryCode : tidy(was.country) !== out.country) out.countryCode = out.country ? countryCodeFor(out.country) : null;
  return out;
}
/** The list the edits will leave, drawn before Apple has it. */
const predicted = (base: PostalAddress[], draft: DraftAddress[]) =>
  draft.map((d) => settled(d.address, d.from === null ? null : base[d.from])).filter((a) => !isEmptyAddress(a));

/**
 * One address being written, laid out as Contacts on the Mac lays one out: the street over
 * as many lines as it takes, then town, region and postcode on one line, then the country.
 */
function AddressEditor({ n, address, custom, autoFocus, onChange, onCustom, onRemove }: {
  n: number;
  address: PostalAddress;
  custom: boolean;
  autoFocus: boolean;
  onChange: (patch: Partial<PostalAddress>) => void;
  onCustom: (custom: boolean) => void;
  onRemove: () => void;
}) {
  const match = ADDRESS_LABELS.find((v) => v === address.label?.toUpperCase());
  const typing = custom || (address.label !== null && !match);
  const field = (part: (typeof ADDRESS_PARTS)[number], extra?: { autoFocus?: boolean }) => (
    <input
      className={`contact-address-${part}`}
      value={address[part] ?? ""}
      placeholder={PART_NAME[part]}
      aria-label={`${PART_NAME[part]}, address ${n}`}
      autoComplete="off"
      autoFocus={extra?.autoFocus}
      onChange={(e) => onChange({ [part]: e.target.value || null })}
    />
  );
  const street = address.street ?? "";
  return (
    <li className="contact-address-edit">
      <span className="contact-address-kind">address</span>
      <Picker
        label={`Label for address ${n}`}
        className="contact-label-pick"
        value={typing ? CUSTOM : (match ?? "")}
        onChange={(v) => {
          onCustom(v === CUSTOM);
          if (v !== CUSTOM) onChange({ label: v || null });
        }}
        options={[
          ...(address.label === null && !typing ? [{ value: "", label: "no label" }] : []),
          ...ADDRESS_LABELS.map((v) => ({ value: v, label: labelText(v) })),
          { value: CUSTOM, label: "Custom" },
        ]}
      />
      {typing ? (
        <input
          className="contact-method-label contact-address-custom"
          value={address.label ?? ""}
          placeholder="label"
          aria-label={`Custom label for address ${n}`}
          onChange={(e) => onChange({ label: e.target.value || null })}
        />
      ) : (
        <span aria-hidden="true" />
      )}
      <button type="button" className="quiet danger" aria-label={`Remove address ${n}`} onClick={onRemove}>
        ✕
      </button>
      <div className="contact-address-fields">
        <textarea
          className="contact-address-street"
          value={street}
          rows={Math.max(2, street.split("\n").length)}
          placeholder="Street"
          aria-label={`Street, address ${n}`}
          autoFocus={autoFocus}
          onChange={(e) => onChange({ street: e.target.value || null })}
          onKeyDown={(e) => {
            // Return saves, as in every other field; Shift-Return starts another line.
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              e.currentTarget.form?.requestSubmit();
            }
          }}
        />
        <div className="contact-address-line">
          {field("city")}
          {field("state")}
          {field("postalCode")}
        </div>
        {field("country")}
      </div>
    </li>
  );
}
