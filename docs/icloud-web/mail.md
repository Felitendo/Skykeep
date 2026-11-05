# Mail (icloud.com `mail2`), static pass 2026-10-19
Bundle: `~/.cache/quince-icloud-re/mail2/pretty/main.js` (line numbers below). Hosts were seen live:
`p{NN}-mccgateway.icloud.com` (mailws2, mailis, mpp) and `p{NN}-mcc.icloud.com` (`cc/mail/v1/account/{dsid}/…`).

## Model
IMAP underneath. Every mailws2 reply carries `sessionHeaders {folder, modseq, threadmodseq, condstore, qresync, threadmode}`
(119943), which is IMAP CONDSTORE/QRESYNC state. The web client keeps `messageModSeq` and `threadModSeq` per folder and
retries `thread/get` up to 3 times on the error `[THREADMODSEQOLD]` (120083). Mail is thread-first. Folders are IMAP paths
(`INBOX`; `/VIP` is client-only and is mapped to INBOX).
INFERRED: the folder is sent in a request header, because the helper takes it as an argument separate from the body.

## Endpoints (all POST JSON to mccgateway unless noted)
| Call | Body (VERIFIED-IN-CODE) |
|---|---|
| `mailws2/v1/thread/search` | `{responseType: "THREAD_DIGEST", includeFolderStatus: true, maxResults, before?, since?, from?, searchText?, searchType?, filters?}`, returning `{threadList[{threadId, timestamp,…}], sessionHeaders, folderStatus}` (40824, 40873) |
| `mailws2/v1/thread/list` | `{responseType: "THREAD_DIGEST", includeFolderStatus: false, threadIds}` (40935) |
| `mailws2/v1/thread/get` | `{threadId, includeLabelIds}` (120083) |
| `mailws2/v1/thread/move` | `{moveMethod: "MOVE" or "DELETE", destFolder, threadIds}` (120385) |
| `mailws2/v1/thread/flag` | `{method: "ADD" or "REMOVE", flags: ["SEEN"] or ["FLAGGED"], threadIds}` (119615) |
| `mailws2/v1/message/send` | `{date (RFC 2822, "-0000"), from, to[emails], cc?, bcc?, subject, textBody, htmlBody, isHME, webmailClientBuild, …}` (120290). From is required. |
| `mailws2/v1/message/{get, list, part, preview, download, savedraft, upload, label/set, checkomit}`, `draft/send` | message-level |
| `mailws2/v1/schedule{,/create,/delete}` | Send Later |
| `mailws2/v1/maildrop/{getMailDropToken, putMailDropRecord, moveAttachmentToMailDrop, republishAttachments}` | Mail Drop (large attachments go through iCloud Drive) |
| `mailws2/v1/{smime/verify, geqs/query, event/list, folder/unseenVip, business/logo, business/merchant_category, feature_enablement/calendar_event_extraction}` | extras |
| `mailis/v1/mail/{sync, sync/VIP, sync/RECOMMENDATIONS, recommendations…, remindmelater, quota}` | categories, Remind Me, VIP |
| `mpp/v1/auth-tokens/{keys/latest, signatures}` | Mail Privacy Protection (remote-content proxy) |
| `GET mcc /cc/mail/v1/account/{dsid}/preference/{web/all, recents, vips}`, `/unified/rule` | settings, rules |
| older `wm/{folder, message, messagepart, preference, vipsender, cloudkit}` | legacy webmail API, still referenced |

## Feasibility for Quince
- iCloud Mail is also standard IMAP/SMTP (`imap.mail.me.com:993`, `smtp.mail.me.com:587`), but that path needs an
  app-specific password. The web API above works with the session Quince already has, so no extra password is needed.
- The work involved: a thread list with modseq-based incremental sync, a thread view (HTML sanitising is required), compose
  and send, flag/move/delete, and search. It is about as big as Notes. Risk: HTML mail rendering in WebKitGTK must be
  sandboxed (no remote content unless allowed, via the mpp proxy).

## Needs live capture
Exact response shapes of `thread/get` and `message/get`; how the folder header is named; how attachments upload; the folder list call.
