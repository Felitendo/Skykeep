# icloud.com design reference (beta builds), light and dark

Taken on 2026-10-17 from the public, unauthenticated bundles on **beta.icloud.com**. Nothing was executed and nobody signed in.

| App | Beta build | Compared with www |
|---|---|---|
| Shell | system/icloud.com 2638BuildBeta18 | www 2638Build27. The theme tokens are identical. Beta moves most shell CSS into lazy chunks 1–9.main.css. |
| Notes | notes3 2638BuildBeta17 | CSS byte-identical to www (md5). JS differs only in build strings. |
| Reminders | reminders2 2638BuildBeta16 | CSS identical to www. JS differs only in build strings. |
| Calendar | calendar 2634BuildBeta23 | CSS differs from www mostly in minifier output (see §8). The app's own styles are **CSS-in-JS** inside main.js. |
| Contacts | contacts2 2638BuildBeta34 | main.css identical to www. Chunks 1/2/4 are shared components. |
| Drive | iclouddrive 2638BuildBeta31 | Changed: new chunks (BrowseToolbar, SharedFilePreview) and new share/request-access screens (§8). |
| Photos | photos3 2638BuildBeta15 | main.css identical to www. |

**How to read citations.** `notes3/main.css:7848` is a line in the formatted copy at
`scratchpad/icloud-bundles/pretty/beta/<app>/<file>`, which has one declaration per line. The formatter is `scratchpad/tools/cssfmt.py`. `calendar/main.js@1034282` is a byte offset in the raw minified bundle `scratchpad/icloud-bundles/beta/<app>/main.js`. The scratchpad is temporary, so re-download if it is gone. URLs: `https://beta.icloud.com/applications/<app>/<build>/en-us/main.css` and `/system/icloud.com/2638BuildBeta18/en-us/main.css`.

Tags: values without a tag were seen in the bundle. **INFERRED** means reasoned from nearby code. **UNKNOWN** means not found statically.

---

## 1. Colour tokens

Every app ships the same theme block, so one copy is cited (`root/main.css`). There are four variants:
- light: `html body`, lines 8–87;
- dark: `@media (prefers-color-scheme:dark) html body`, lines 88–155;
- forced dark: `[data-appearance=dark]`, lines 156–223;
- forced light: `[data-appearance=light]`, lines 224–300.

The same block appears in `notes3/main.css:9895`, `reminders2/main.css:780`, `calendar/main.css:2519`, `contacts2/main.css:1210` and `photos3/main.css:1`. Hex values in brackets are converted from the HSL triplets.

### 1.1 Text (labels)
| Token | Light | Dark | Source |
|---|---|---|---|
| labelPrimary | rgba(0,0,0,0.88) | hsla(0,0%,100%,0.98) | root/main.css:9 / :89 |
| labelSecondary | rgba(0,0,0,0.56) | hsla(0,0%,100%,0.66) | :10 / :90 |
| labelTertiary | rgba(0,0,0,0.48) | hsla(0,0%,100%,0.5) | :11 / :91 |
| labelQuaternary | rgba(0,0,0,0.32) | hsla(0,0%,100%,0.42) | :12 / :92 |

### 1.2 Fills (hover, pressed, tracks)
| Token | Light | Dark | Source |
|---|---|---|---|
| fillPrimary | rgba(120,120,128,0.2) | hsla(0,0%,71.4%,0.3) | :13 / :93 |
| fillSecondary | rgba(120,120,128,0.16) | rgba(120,120,128,0.36) | :14 / :94 |
| fillTertiary | rgba(118,118,128,0.12) | rgba(118,118,128,0.3) | :15 / :95 |
| fillQuaternary | rgba(120,120,128,0.08) | rgba(116,116,128,0.25) | :16 / :96 |

### 1.3 Greys
| Token | Light | Dark | Source |
|---|---|---|---|
| gray1 | hsl(240,2.3%,56.7%) [#8e8e93] | same | :17–20 |
| gray2 | #aeaeb2 | #636366 | :21 / :101 |
| gray3 | hsl(240,4.7%,79%) [#c7c7cc] | hsl(240,1.4%,29%) [#49494b] | :22–25 / :102–105 |
| gray4 | hsl(240,5.7%,82.9%) [#d1d1d6] | hsl(240,1.6%,23.9%) [#3c3c3e] | :26–29 / :106–109 |
| gray5 | hsl(240,10.6%,90.8%) [#e5e5ea] | hsl(240,1.9%,20.8%) [#343436] | :30–33 / :110–113 |
| gray6 | #f2f2f7 | #2c2c2e | :34 / :114 |

### 1.4 Backgrounds and surfaces
| Token / use | Light | Dark | Source |
|---|---|---|---|
| backgroundPrimary: content, menus, popovers, sheets, list panes | hsl(0,0%,100%) [#fff] | hsl(240,3.4%,11.4%) [#1c1c1e] | :66–70 / :140–144 |
| backgroundLayer1: app toolbar (`.cloudos-toolbar.app`) | #f2f2f7 | #323236 | :71 / :145 |
| backgroundLayer2 | #fff | #434349 | :72 / :146 |
| sidebar: Notes folder panel, Calendar cells | #fbfbfd | #202023 | :82 / :151 |
| divider: Reminders sidebar edge, Notes slide-out | #d1d1d6 | #000 | :83 / :152 |
| separator: row hairlines, list headers | #d1d1d6 | #343436 | :84 / :153 |
| backdrop: modal veil | rgba(245,245,247,0.85) | rgba(10,10,10,0.8) | :81 / :150 |
| boxShadow colour | rgba(0,0,0,0.16) | rgba(0,0,0,0.65) | :77 / :149 |
| focusRing = tintBlue | rgba(0,113,227,0.7) | rgba(10,132,255,0.7) | :59,:80 / :133 |
| App toolbar bottom border | 1px gray5 | 1px gray5 | calendar/main.css:4329 |
| Split-view divider (sidebar to content) | 1px gray4 | 1px gray4 | calendar/main.css:5319 |

### 1.5 System colours
| Token | Light | Dark | Source |
|---|---|---|---|
| systemBlue | #0071e3 | #009aff | :46 / :126 |
| systemRed (destructive) | hsl(0,100%,44.5%) [#e30000] | hsl(358,100%,59.4%) [#ff3037] | :36–39 / :116–119 |
| systemGreen | #03a10e | #32d158 | :42 / :122 |
| systemOrange | #ff9500 | #ff9f0a | :40 / :120 |
| systemYellow | #fc0 | #ffd60a | :41 / :121 |
| systemPink | #ff2d55 | #ff375f | :35 / :115 |
| systemPurple | #af52de | #bf5af2 | :48 / :128 |
| systemIndigo | #5856d6 | #5e5ce6 | :47 / :127 |
| systemTeal / Mint / Cyan / Brown | #30b0c7 / #00c7be / #32ade6 / #a2845e | #40c8e0 / #63e6e2 / #64d2ff / #ac8e68 | :43–49 / :123–129 |
| tintBlueOpaque (button focus ring) | #4b9beb | #0f64bb | :60 / :134 |
| tintBlueSecondary / Tertiary | rgba(0,113,227,.28) / .12 | rgba(10,132,255,.28) / .12 | :63–66 |
| tintRed (invalid fill) | rgba(227,0,0,0.32) | rgba(244,10,10,0.48) | :61 / :135 |
| tintNotes | #ebb800 | not overridden in dark | :55 |

### 1.6 Accent per app (`--theme-color-appTint`)
| App | Light | Dark | Source |
|---|---|---|---|
| Default: Drive, Reminders, Contacts, Photos, Keynote | hsl(210,100%,44.5%) [#0071e3] | hsl(204,100%,50%) [#0099ff] | root/main.css:303–315; reminders2/main.css:1075 |
| Calendar | hsl(0,100%,44.5%) [#e30000] | hsl(358,100%,59.4%) [#ff3037] | calendar/main.css:4144, :4157 |
| Notes | hsl(47,100%,46.1%) [#ebb800], the same in dark | same | notes3/main.css:9128, :9160 |
| Notes `--key-color` (folder icons, focused selection, buttons) | rgb(235,184,0); hover #d1a400; active #ad8900; focused-hover #c09c34, with text #d6d6d6 | same | notes3/main.css:9136–9141 |
| Drive Numbers / Pages | hsl(124,96.3%,32.2%) / hsl(35,100%,50%) | hsl(134,63.3%,50.8%) / hsl(36,100%,52%) | iclouddrive/main.css:867–900 |
| Reminders list tint (`--tint-color`) | the list's own colour; title, buttons and the done tick use it | same | reminders2/main.css:7254, :7069, :7990 |

### 1.7 State model (shared)
These values come from root/main.css:73–76 and :147–148.
- **Hover** darkens lightness by 8% (`--theme-darken-background-hover:8%`). In dark it lightens by 8% (`-8%`).
- **Pressed** changes lightness by 16%.
- **Pressed tint background** is the accent at 0.16 alpha. **Pressed text** is the accent at 0.3 alpha.
- **Disabled** is opacity 0.3 in light and 0.4 in dark (calendar/main.css:4755–4766).

### 1.8 Selection colours
| App / element | Hover | Selected, unfocused | Selected, focused | Source |
|---|---|---|---|---|
| Notes folder row | fillQuaternary | fillSecondary | key colour, white text and icon; hover #c09c34 | notes3/main.css:9152–9156 vars, :7343, :7371, :7377, :7384 |
| Notes note row | none | fillSecondary | key colour, white text, snippet white | :7661, :7670, :7677, :7910 |
| Notes row with menu open | | | appTint at .3 alpha, white text | :7656, :7361 |
| Reminders list (sidebar) | fillQuaternary (`--unselected-hover-color`) | fillQuaternary (`--selection-color`); pressed fillTertiary | appTint, white text; hover tint −8%; active −16% | reminders2/main.css:10711–10726, :5941–5974 |
| Reminders reminder row | | fillQuaternary | fillTertiary (`--row-highlight-color`). Note `fillQuarternary` is misspelt in the source, so the unfocused value resolves to nothing. | :7672, :7686, :7864 |
| Drive sidebar item | fillQuaternary | gray5 | appTint, white text and icon | iclouddrive/11.main.css:356, :363, :369 |
| Drive list row | fillQuaternary, radius 10 | gray4 | appTint, white; hover `--app-accent-color-hover` (tint −8%) | iclouddrive/7.main.css:43, :46; 14.main.css:423, :426 |
| Photos sidebar | fillQuaternary | fillTertiary | systemBlue | photos3/ui.main.css:12880, :12905, :12979 |
| Menu item highlight | fillTertiary (grey, **not** the accent) | | | calendar/main.css:5562 |

## 2. Typography

**Font stack.**
- Non-Apple browsers, including Linux: `SF Pro Text, Helvetica Neue, sans-serif` for text and `SF Pro Display, …` for large titles. These are web fonts from `//www.apple.com/wss/fonts/?families=SF+Pro,v3`, linked from each app page.
- Notes uses `SFUIText` / `SFUIDisplay` (notes3/main.css:9134, :2897).
- Apple devices get `system-ui,-apple-system,BlinkMacSystemFont,…`. That stack applies only when `body[apple-system-font-capable]` is set, which JS does only on macOS ≥ 10.14 or iOS ≥ 12 (root/main.js@1362076; root/main.css:317).
- Body: 15px, weight 400 (root/main.css:303, reminders2/main.css:1075, calendar/main.css:2814). Reminders uses `.9375rem`. Notes legacy body is 15px with line-height 1.2 (notes3/main.css:12174).
- Notes body adds `font-feature-settings:"ss07"` (notes3/main.css:9132).
- **Letter-spacing: none.** Across all five apps, letter-spacing is set only to `normal`, `0` or `-.25px`, a handful of times in total. The tracking comes from the SF Text and SF Display optical cuts, not from CSS.

**Type scale.** This is the only explicit scale; it comes from Calendar's CSS-in-JS (calendar/main.js@1034282):
```
sizes   micro 7 · tiny 11 · small 13 · medium 14 · regular 15 · large 17 · big 24 · bigger 26 · huge 34
weights ultralight 100 … regular 400 · medium 500 · semibold 600 · bold 700 · heavy 800 · black 900
```

| Role | Size / weight / line-height | Source |
|---|---|---|
| App title in the toolbar | 21px / 600, SF Pro Display | calendar/main.css:4226–4238 |
| Reminders list title | 2rem (32px) / 700 / 2.3125rem, in the list colour | reminders2/main.css:7283 |
| Drive folder title | 24px / 500 / 30px | iclouddrive/main.css:4858 |
| Notes editor Title / Heading / Subheading / Body (Aa menu previews) | 28/700 Display · 22/700 Display · 17/700 · 17/400 | notes3/main.css:2897–2921 (the editor itself is canvas-rendered; INFERRED to match) |
| Notes nav-bar title | 17px / 600, line-height 52px | notes3/main.css:9571 |
| Alert title / body | 19px / 600 / 1.4 · 14px labelSecondary / 1.4 | calendar/main.css:2903, :2916 |
| Card (sheet) header | 17px / 600 | calendar/main.css:8241 |
| List row primary (note title, Drive name, reminder title) | 17px / 600 / 21px (Notes) · 17px / 400 / 30px (Drive) · 1.0625rem / 1.375rem (Reminders) | notes3/main.css:7848; iclouddrive/7.main.css:72, :94; reminders2/main.css:8005 |
| List row secondary | Notes date + snippet 15px / 400 / 20px, snippet labelSecondary · Drive second row 14px labelTertiary · Reminders notes 1rem/1.25rem (.9375rem at ≥668px) labelSecondary | notes3/main.css:7872, :7900; iclouddrive/7.main.css:104; reminders2/main.css:8063, :8081 |
| Sidebar rows | 15px / 400 in Notes, Drive, Reminders and Photos | notes3/main.css:7139; iclouddrive/11.main.css:209; reminders2/main.css:6098; photos3/ui.main.css:12780 |
| Sidebar section header | 14px / 600 labelSecondary (Drive) · 14px (Photos) | iclouddrive/11.main.css:230; photos3/ui.main.css:12936 |
| Sidebar counts | 1.0625rem labelTertiary (Reminders) | reminders2/main.css:6297 |
| Notes list section header ("Today") | 15px / 700, labelSecondary | notes3/main.css:8029 |
| Drive list column headers | 14px / 400 labelTertiary | iclouddrive/14.main.css:209, :288 |
| Menu items, buttons, segmented, text fields | 14px | calendar/main.css:5486, :4748, :8549, :1277 |
| Calendar month day number | 15px / 400 (30×30 box) | calendar/main.js@2406855 |
| Calendar event text | 12px / 400, line-height 14px | calendar/main.js@2409013, @1884717 |
| Calendar week header | day number 15/600, weekday 17/400 labelTertiary | calendar/main.js@2418958 |
| Calendar now-line label | 13px, systemRed | calendar/main.js@2398430 |

## 3. Spacing and sizes

| Element | Value | Source |
|---|---|---|
| App toolbar (`.cloudos-toolbar`) | height 44px, padding-inline 16px 6px; action buttons 36×36, radius 8, 4px apart | calendar/main.css:4174, :5027 |
| Notes nav bar | 52px line box; buttons 28×28, radius 8 | notes3/main.css:9571, :9618 |
| Drive nav bar | padding 5px 15px 5px 10px; title padding-inline 9px | iclouddrive/main.css:4713, :4858 |
| Reminders sidebar width | 280px, 1px divider on the trailing edge, padding 16px 10px 0 | reminders2/main.css:5742, :5790 |
| Notes / Drive / Calendar sidebar width | UNKNOWN: set by the JS split container, not CSS | — |
| Sidebar row height | 32px for Notes folder, Drive source item and Reminders list (2rem) | notes3/main.css:7139; iclouddrive/11.main.css:316; reminders2/main.css:5895 |
| Sidebar padding | Notes folder list 10px 10px 0; rows padding-inline-end 12px | notes3/main.css:7405, :7139 |
| Sidebar icons | Notes folder 17.4×14; Drive 20px wide with 9px margin; disclosure chevron 10px | notes3/main.css:7238; iclouddrive/11.main.css:399; notes3/main.css:7292 |
| Notes note row | inset 10px each side (width calc(100% − 20px)), padding 10px; gutter 6px | notes3/main.css:7625, :7724 |
| Notes row separator | 1px, starting 33px from the row's leading edge, hidden next to the selection | notes3/main.css:7691, :7703 |
| Reminders row | padding-inline-start 14px (48px when nested); content padding 11px top and bottom, 10px end | reminders2/main.css:7689, :7806, :7920 |
| Reminders checkbox | 1.375rem (22px) circle, 12px gap before the title | reminders2/main.css:7954, :7996 |
| Reminders separator | starts at 12px + 1.375rem (34px) from the content edge | reminders2/main.css:7778 |
| Reminders list header | padding-inline 24px 10px; footer 24px | reminders2/main.css:7015, :6977 |
| Drive list rows | 41px single-line (`thin-rows`), 61px two-line (`thick-rows`); header row 44px; tbody padding-inline 20px (10px at ≤667px); list top padding 44px (20px when compact) | iclouddrive/14.main.css:326–390, :209, :271 |
| Calendar month cell | padding 6px; number box 30×30 at top left; event chip height 20px | calendar/main.js@2406855, @1942374 (`aL=20`) |
| Push button | height 30, min-width 30, padding-inline 10; large 36; icon 28×28 with a 17px svg | calendar/main.css:4685–4851 |
| Block button | height 30, padding-inline 16; large 36 | calendar/main.css:4474, :4535 |
| Text field | 36px tall, 180px default width, padding 0 10px, line-height 18px | calendar/main.css:1277, :1291 |
| Alert | compact max-width 335, padding 40px 0 20px · regular 450 wide, padding 40/40/30; icon 36px | calendar/main.css:2931–2957, :2886 |
| Popup (modal) | padding 20, margin 20, max-width 640, max-height 100vh − 40 | calendar/main.css:3407 |
| Card (bottom sheet) | max-width 600, min-height 200; header padding 20px 20px 10px | calendar/main.css:8181, :8200 |
| Menu | padding 5; min-width 130; item line-height 30, padding 0 10; leading icon box 30×30; divider 9px tall, 1px line inset 11px each side | calendar/main.css:5425, :5486, :5512, :5622, :5637 |

## 4. Shapes (corner radii)

| Element | Radius | Source |
|---|---|---|
| Menu | 12px; items 8px | calendar/main.css:5425, :5486 |
| Popover | 12px; arrow 16×10 | calendar/main.css:6937, :6989 |
| Popup / modal dialog | 11px (Calendar's availability popup is 16) | calendar/main.css:3407; calendar/main.js@2285473 |
| Card (sheet) | 10.5px 10.5px 0 0 (bottom sheet) | calendar/main.css:8181 |
| Buttons (push, block, toolbar) | 8px; focus inner ring 5px | calendar/main.css:4685, :4440, :4791 |
| Segmented | track 8px, segment 6px | calendar/main.css:8549, :8594 |
| Text field | 8px | calendar/main.css:1277 |
| Switch | pill (8.5px on a 17px track) | calendar/main.css:9009 |
| Checkbox | hit area 8px; the box itself is an SVG | calendar/main.css:6037 |
| Sidebar row selection | 10px (Notes, Drive, Reminders, Photos) | notes3/main.css:7139; iclouddrive/11.main.css:316; reminders2/main.css:5881; photos3/ui.main.css:12905 |
| List row selection | Notes 8px; Drive 10px (first and last of a contiguous run only); Reminders 10px | notes3/main.css:7625; iclouddrive/7.main.css:31–39; reminders2/main.css:7689 |
| Calendar event chip / timed block | 4px | calendar/main.js@2345946, @2346678 |
| Calendar today marker | 50% (30px circle) | calendar/main.js@2406855 |
| Reminders detail fields | 8px | reminders2/main.css:8857 |

## 5. Shadows and blur

| Element | Light | Dark | Source |
|---|---|---|---|
| Standard elevation (`--theme-boxShadow-standard`) | 0 11px 34px rgba(0,0,0,.16) | 0 11px 34px rgba(0,0,0,.65) | root/main.css:77–79 |
| Menu | standard; no border | standard (dark) | calendar/main.css:5425, :5447 |
| Popover | standard plus a .2px border in fillQuaternary | standard (dark) | calendar/main.css:6937, :6948 |
| Popup / modal | standard | standard (dark) | calendar/main.css:3407, :3453 |
| Card (sheet) | 0 0 15px rgba(0,0,0,.15) | same | calendar/main.css:8181 |
| Selected segment | 0 3px 8px fillTertiary, plus a 1px outline in fillQuaternary | 0 3px 8px rgba(0,0,0,.3) | calendar/main.css:8655, :8663 |
| Switch knob | 0 2px 4px fillPrimary, 0 2px 1px fillTertiary, .5px outline fillTertiary | same tokens | calendar/main.css:9048 |
| Sidebar | none: flat `--theme-color-sidebar` (Notes) or backgroundPrimary with a 1px divider | — | notes3/main.css:3310; reminders2/main.css:5742 |
| **Backdrop-filter** | Not used on any menu, popover, sheet or sidebar in Notes, Reminders, Calendar or Drive. The only uses are the homepage toolbar (blur 14px), the app-switcher popover (blur 50px), homepage tiles, the compact phone card (blur 15px saturate 86%) and the Notes enlarged scroll indicator (blur 10px). | | calendar/main.css:4263; root/main.css:12753, :5919; notes3/main.css:8105 |
| Modal backdrop | rgba(245,245,247,.85), a **light** frosted veil with no blur | rgba(10,10,10,.8) | calendar/main.css:3390 |

## 6. Motion

Keyframes are defined at calendar/main.css:8929–9004 and are the same in every app.

| Interaction | Duration / easing | Keyframes | Source |
|---|---|---|---|
| Context menu open and close | none: appears and disappears instantly (no CSS animation; the JS uses no `animate()`) | — | INFERRED; calendar/main.css has no `ui-menu` open rule |
| Select-button menu | in: fadeIn .2s ease · out: fadeOut .2s ease | opacity | calendar/main.css:6607, :6611 |
| Popover | in: .2s cubic-bezier(.25,.1,.25,1.3), which overshoots · out: fadeOut .2s ease | fadeInAndScale: scale .5→1, opacity 0→1 | calendar/main.css:7066, :7070, :8965 |
| Popup (modal) | in and out .2s cubic-bezier(.32,.08,.24,1) | fadeInAndBarelyScale: scale .9→1 · fadeOut | calendar/main.css:3513, :3517, :8992 |
| Alert | .2s ease-in-out (fade only under reduced motion) | scaleWithBriefFade: scale .5→1, opacity done by 80% | calendar/main.css:2995, :3000, :8977 |
| Backdrop | .2s cubic-bezier(.32,.08,.24,1); with a card .35s cubic-bezier(0,0,0,1) | fadeIn / fadeOut | calendar/main.css:3390–3399, :8283 |
| Card (sheet) | .35s cubic-bezier(0,0,0,1) | slideIn translateY 100%→0 · slideOut | calendar/main.css:8279, :8287 |
| Switch | .25s, background-color and knob inset-inline-start (knob x 1→12px) | — | calendar/main.css:9009, :9032, :9042 |
| Legacy segmented / text field | .25s colour / box-shadow | — | calendar/main.css:872, :736 |
| Scrolled-under hairline (Notes list, folder list) | border-color .12s | — | notes3/main.css:8116, :3329 |
| Reminders group disclosure chevron | transform .2s | — | reminders2/main.css:6323 |
| Image fade-in | opacity .2s ease | — | calendar/main.css:5199 |
| Calendar availability border | border-image .2s linear | — | calendar/main.js@2283854 |
| Selection change, list insert and remove, sidebar collapse | no CSS transition: these are instant | — | INFERRED (none found in any app CSS) |
| Spinner | 1s, eight nibs staggered .125s, opacity .1→.8 | — | calendar/main.css:3683–3745 |

## 7. Components

### Menu (`ui-menu.standard.icloud-mouse`)
- **Surface:** backgroundPrimary, radius 12, padding 5, min-width 130, max-width 100vw − 20, standard shadow (.16 light, .65 dark), no border, no blur. Source: calendar/main.css:5425.
- **Items:** 14px text in labelPrimary, line-height 30, padding 0 10, radius 8 (:5486).
  - Hover/keyboard highlight is fillTertiary. Keyboard focus adds an inset 3px tintBlue ring (:5562, :5565).
  - Pressed: background appTint @.16, icon appTint @.3 (:5996).
  - Disabled: labelQuaternary (:5552).
  - Leading icons are filled with appTint (:5993).
- **Destructive item:** systemRed text and icon; pressed adds a fillSecondary background (:5586–5592).
- **Link item:** systemBlue (:5601).
- **Checkmark:** a 16px icon at the trailing end, in appTint (:5614, :6004).
- **Separator:** 9px tall, 1px fillTertiary line inset 11px each side (:5622, :5637).
- **Overflow:** 18px scroller bands with 8px arrows (:5653).

### Popover (`ui-popover.standard`)
- Content uses backgroundPrimary, radius 12, overflow hidden, a .2px fillQuaternary border and the standard shadow (calendar/main.css:6937).
- The arrow is 16×10 and filled with backgroundPrimary (:6989, :6998).
- An actions footer uses a 1px gradient border in gray5 that fades out at both ends, with 19px top padding (:6969, :6986).

### Sheets
- **Popup (centred modal):** see §3–§6; radius 11, padding 20.
- **Close button:** 14px glyph in labelPrimary, radius 8 (:3422, :3447).
- **Card:** slides up from the bottom; radius 10.5 at the top only.
- **Alert:** centred title 19/600 and description 14 (:2903).

### Segmented control (`ui-segmented-control.standard`)
- **Track:** padding 2, fillTertiary background, radius 8, 14px labelPrimary text. Hover and pressed change it to fillSecondary. Focus adds a 3px tintBlue ring. Source: calendar/main.css:8549–8590.
- **Segments:** 24px tall, min-width 78, padding 0 16, radius 6 (:8594).
- **Separators:** 1×16px gray3, shown only between two unselected segments (:8632, :8652).
- **Selected segment:** backgroundPrimary in light, **gray2 (#636366) in dark**; font-weight 600; 1px outline in fillQuaternary; shadow 0 3px 8px fillTertiary (dark rgba(0,0,0,.3)) (:8655–8684).
- **Pressed segment:** fillQuaternary (:8696).

### Toggle (`ui-switch.standard`)
- **Track:** 28×17, radius 8.5. Off is gray4 (#d1d1d6 / #3c3c3e). **On is appTint, not green.** Source: calendar/main.css:9009, :9066, :9256.
- **Knob:** 15px white, 1px from the top; x is 1px off and 12px on (:9028–9048).
- **States:** hover and pressed add an inset 0 0 28px labelQuaternary shadow. Focus adds a 0 0 0 3px tintBlue ring. Disabled is opacity .3 (:9069–9088).
- **Labelled variant:** 32×17, with an "I/O" glyph (:9121).

### Checkbox (`ui-checkbox.standard`)
- **Box:** 16×16 SVG. Unchecked: a square stroked in gray3. Checked: the square fill (`checkmark-square-fill`) in appTint with the checkmark drawn over it. Source: calendar/main.css:6079–6128, :6195.
- **Hit area:** a 16px box with 6px padding, radius 8. Hover gives it fillTertiary. Focus adds fillTertiary plus an inset 3px tintBlue ring. Pressed: appTint @.16 (:6057, :6160–6198).
- **Labelled variant:** padding 7×12, 14px label (:6219, :6244).
- **Calendar list checkbox:** the fill is the calendar colour, and the checkmark is white or dark depending on contrast (calendar/main.js@2520199).
- **Reminders checkbox:** a 22px circle glyph. Unchecked it is labelTertiary; completed it is filled in the list colour (reminders2/main.css:7990–7996).

### Search field
There is no shared search field.
- **Notes:** `.notes-list-search-field` in notes3/main.css. The full metrics were not extracted (UNKNOWN).
- **Drive:** `.skybridge-searchbox` in iclouddrive/9.main.css (not re-fetched; UNKNOWN).
- **Legacy `cw-search-field`:** 36px tall, 17px text, 16px glass icon in #ccc at 10px inset, padding-inline 31 (calendar/main.css:927–1004). It is legacy and should not be used as the model.
- **Text field (`ui-text-field.standard`):** 1px gray3 border, radius 8, 36px tall, 14px text, placeholder labelTertiary. Hover overlays fillQuaternary. Focus removes the border and draws an inset 3px tintBlue ring, and the placeholder becomes labelQuaternary. Disabled is opacity .3. Source: calendar/main.css:1277–1508.

### Buttons
| Variant | Values | Source |
|---|---|---|
| Push, default (borderless) | 30px tall, radius 8, padding-inline 10, 14px, text appTint. Hover: fillTertiary. Pressed: background appTint @.16, text appTint @.3. | calendar/main.css:4685–4918 |
| Push primary / secondary | weight 600 / 400 | :4828, :4831 |
| Push destructive | systemRed text, no transition; pressed fillPrimary background, red @.3 | :4964, :4969 |
| Push "pressed" (toggled on) | appTint background, white text | :4922 |
| Block primary (filled) | appTint background, white text, radius 8, 30px, padding 16, 14px; hover tint −8% L, pressed −16% | :4669–4679, :4474 |
| Block secondary | gray5 background in light, gray4 in dark; labelPrimary text; hover/pressed darken | :4596–4619 |
| Block secondary destructive | gray5/gray4 background, systemRed text | :4624 |
| Block neutral | gray1 background, white text | :4580 |
| Toolbar icon button | 36×36, radius 8, labelPrimary; hover fillSecondary; pressed fillPrimary; open gray3 background with backgroundPrimary glyph | :5027–5066 |
| Notes toolbar button | 28×28, radius 8; icon in key colour; pressed: key colour @.16 background, icon @.3; menu open: key-colour background, white icon | notes3/main.css:9618–9665 |
| Focus (all buttons) | inset 0 0 0 3px tintBlueOpaque plus a 1px white inner ring (gray5 in dark) at 3px, radius 5 | calendar/main.css:4773–4791 |

### List rows
| App | Normal | Hover | Selected, focused | Selected, unfocused |
|---|---|---|---|---|
| Notes note | backgroundPrimary; padding 10; radius 8; title 17/600; summary 15; separator from 33px | none | key colour (yellow), all text white | fillSecondary |
| Notes folder | 32px, radius 10, 15px; yellow folder glyph | fillQuaternary | key colour, white | fillSecondary |
| Reminders list | 32px (2rem), radius 10, 15px; count 17px labelTertiary | fillQuaternary | appTint, white | fillQuaternary |
| Reminders reminder | 22px circle, 17px title, 11px vertical padding, radius 10, separator from 34px | — | fillTertiary | fillQuaternary (as intended; see §1.8) |
| Drive file | 41px (61px two-line), name 17px, meta 14px labelTertiary, separator hairline | fillQuaternary, radius 10 | appTint, white | gray4 |
| Drive sidebar | 32px, radius 10, 15px, appTint glyph | fillQuaternary | appTint, white | gray5 |

## 8. Calendar specifics (CSS-in-JS in calendar/main.js)

- **Theme constants** map straight onto the CSS variables (@1880470): `Fk=labelPrimary`, `Uk=labelTertiary`, `Jk=systemRed`, `Qk=backgroundPrimary`, `Xk=backgroundLayer1`, `fR="1px solid "+backgroundLayer1`, `uR=sidebar`.
- **Month cell:**
  - Top border 1px backgroundLayer1. The highlighted cell background is `--theme-color-sidebar` (@2406614). INFERRED: this marks the weekend or the selection.
  - The day number sits at top left in a 30×30 box, 15/400. In-month numbers are labelPrimary; out-of-month numbers are gray5.
  - **Today** is a filled systemRed circle with backgroundPrimary text and a 6px gap (@2406855).
- **Week header:** today's number sits in a 30px red circle (15/600). The weekday name is 17px labelTertiary (@2418958).
- **Event chip in month view:**
  - 20px tall, radius 4, padding-inline 5, 12px regular (@2409013, @1942374).
  - **Background:** the calendar colour at **20% alpha**, or 90% when selected (`YB`, @2007511).
  - **Text:** a darkened variant per calendar colour: red #932f40, orange #a24311, yellow #a24311, green #558d2f, blue #2a6193, purple #7e4d82, brown #865033 (@2008437).
- **Timed event block:** padding 3, radius 4, a **3px left border** in the calendar colour, 12px text with line-height 14 (@2346678).
- **Now line:** systemRed. Its label is 13px, right-aligned, 10px from the gutter edge (@2398430).
- **beta vs www (calendar/main.css):**
  - Beta moves `border-radius:13px` on `.cw-alert.cw-modal-dialog:before` to touch mode only (www: all modes; beta: `[data-primary-interaction-mode=touch]`).
  - The monogram font is now `SFUIText,Helvetica Neue` with a `system-ui` override.
  - Everything else is minifier noise, for example `#c70a00` written as `rgb(199,9.5,0)`.
  - The JS grew by about 14KB. The design tokens are unchanged.

## 9. Drive and Photos (brief)

- **Drive beta vs www.**
  - Same values in SourceList, PathBar, Search, Move and the Grid/List leaf chunks (0 differing lines).
  - The shared list chunk lost the `[data-overflow-fade]` utility (55 lines moved).
  - QuickLook differs by 824 lines and main.css by 1280. These are new request-access, join-share, edit-gate and restricted-unauth screens (beta main.css). BrowseToolbar (8.main.css) and SharedFilePreview (15.main.css) are beta-only.
  - No token values changed.
- **Photos:**
  - The sidebar is a `tree-view.sidebar` with a fillQuaternary background. Items are 15px, radius 10. Hover fillQuaternary, selected fillTertiary, focused systemBlue (photos3/ui.main.css:12763–12979).
  - Section titles are 14px.
  - Photos-specific variables: sidebar-material-fill hsla(0,0%,100%,.85) in light and rgba(40,40,40,.5) in dark; sidebar border #e6e6ea / #333 (photos3/main.css:396–400, :510–514).
- **Contacts:** the app's own styles use hashed CSS-module class names (`._1SIfy45…`) that map to components only through JS. They were not resolved, so Contacts row and card metrics are UNKNOWN. Its controls are the shared components above.
- **Reminders has no smart-list tiles on icloud.com.** The CSS has no tile classes, and the JS has no Today/Scheduled/Flagged list keys (only `Date.Today` and `Reminder.Flagged.VoiceOver`). The sidebar is a plain list of lists.

---

## 10. Quince vs icloud.com

Ranked by how visible each difference is. Quince files are in `src/styles/`. Rows marked **DELIBERATE** contradict a comment in Quince's own CSS, so they are listed for completeness and should not be changed without asking.

### High visibility
| # | What | Quince now | icloud.com | Where to change |
|---|---|---|---|---|
| 1 | Accent | one gold for every app: `--gold #c8912a` | per app: Notes #ebb800, Calendar #e30000 / #ff3037, Drive, Reminders and Contacts #0071e3 / #0099ff | tokens.css `:root --gold*`. **DELIBERATE** (tokens.css:1–4). |
| 2 | Focused selection | tinted: `.nav-item[aria-current] gold-tint 13%`, `.note-row[aria-current] gold 26%`, `.rx-list[aria-current] text 12%`, `.notes-folder[aria-current] text 12%` | **solid accent with white text** when the list has focus; fillSecondary (Notes) / fillQuaternary (Reminders) / gray5 (Drive) when it does not | notes.css `.note-row[aria-current]`, `.notes-folder[aria-current]`; reminders.css `.rx-list[aria-current]`; app.css `.nav-item[aria-current]`, `.drive-row.is-selected` (Drive already fills with `--sys-blue`, which matches) |
| 3 | Base text size | body 13px (`--t-13`); rows, sidebar and menus 13px | body 15px; sidebar rows 15px; list primary 17px; menus, buttons and fields 14px | tokens.css `body font-size`, and the `--t-13` uses in row selectors. **DELIBERATE** (tokens.css:30–31). |
| 4 | Sidebar material | floating glass plateau: `.sidebar` blur(28px) saturate(180%), radius 10, 8px inset, 196px | flat and opaque: `--theme-color-sidebar` #fbfbfd / #202023 (Notes) or white, a 1px gray4/divider edge, no blur, no radius; Reminders 280px | app.css `.sidebar`. **DELIBERATE** (app.css:252–253, the "Liquid Glass" chrome). |
| 5 | Sidebar row shape | `.nav-item` min-height 34, radius 7; `.notes-folder` 30 / 7; `.rx-list` 32 / 7; `.cx-cal` 26 / 6 | 32px, **radius 10**, 15px | app.css `.nav-item`; notes.css `.notes-folder`; reminders.css `.rx-list`; calendar.css `.cx-cal` |
| 6 | Notes list row | `.note-row` padding 8px 10px 9px, radius 8; title 13/700; meta 11 | padding 10, radius 8, title **17/600/21**, date and snippet **15/400/20**, snippet labelSecondary, separator from 33px | notes.css `.note-row`, `.note-row-title`, `.note-row-meta` |
| 7 | Reminders row | `.rx-tick` 20px, ring text @32%; `.rx-title` 13px / 20px; padding 7px | **22px** circle in labelTertiary, title 17px / 22px, 11px vertical padding, separator from 34px | reminders.css `.rx-tick`, `.rx-title`, `.rx-row`, `.rx-row::after left` |
| 8 | Reminders sidebar tiles | `.rx-tiles` 2×N tile grid | none on icloud.com: a plain list of lists (the tiles are a macOS/iOS feature) | reminders.css `.rx-tiles` (a design choice; keep it if macOS parity is the goal) |
| 9 | Context and row menus | `.row-menu` radius 10, item `5px 10px` radius 6; highlight **`--sys-blue` fill, white text**; `.picker-menu li.is-active` gold fill | radius **12**, padding 5, items 30px line, radius **8**, 14px; highlight **fillTertiary grey**, text stays labelPrimary; destructive in systemRed text; shadow 0 11px 34px | app.css `.row-menu`, `.row-menu-item.is-active`, `.picker-menu`, `.picker-menu li.is-active`, `.upload-menu button:hover` |
| 10 | Modal veil | `.modal-veil` rgba(0,0,0,.32) + blur(2px) | light rgba(245,245,247,.85) / dark rgba(10,10,10,.8), no blur | app.css `.modal-veil` |
| 11 | Drive list rows | `.drive-row-hit` padding 8px 10px, 13px (about 36px tall); meta 11px | 41px rows (61px two-line), name 17px, meta 14px labelTertiary; hover fillQuaternary radius 10; header 44px, 14px labelTertiary | app.css `.drive-row-hit`, `.drive-meta`, `.drive-head`, `.drive-col` |
| 12 | Calendar month cell | `.cx-daynum` top **right**, 13px in a 22px pill; weekday names right-aligned 13px | day number top **left**, 15px in a **30px** circle; today a red filled circle; out-of-month gray5 | calendar.css `.cx-daynum`, `.cx-daynum.is-today`, `.cx-weekdays span` |
| 13 | Calendar chips and blocks | `.cx-item` 18px tall, 11px, radius 4, dot + title; `.cx-bar-event` cal @24%; `.cx-block` cal @22% mixed with bg, radius 5, 11px | chip **20px**, **12px**, padding-inline 5, background cal @**20%** with a darkened per-colour text (#932f40 …); block radius **4**, 3px left border, 12px | calendar.css `.cx-item`, `.cx-bar-event`, `.cx-block`, `.cx-block-title` |

### Medium visibility
| # | What | Quince now | icloud.com | Where |
|---|---|---|---|---|
| 14 | Text colours | `--text #1d1d1f`, `--text-2 #6e6e73`, `--text-3 #8e8e93`; dark #f5f5f7 / #a1a1a6 / #8e8e93 | alpha labels: rgba(0,0,0,.88 / .56 / .48 / .32); dark hsla(0,0%,100%,.98 / .66 / .5 / .42) | tokens.css `--text`, `--text-2`, `--text-3` |
| 15 | Backgrounds | `--bg #f5f5f7` / `#1c1c1e`; `--sheet #fff` / `#2c2c2e` | content #fff / #1c1c1e; sidebar #fbfbfd / #202023; toolbar #f2f2f7 / #323236; menus and sheets #fff / **#1c1c1e** (not #2c2c2e) | tokens.css `--bg`, `--sheet` |
| 16 | Separators | `--separator rgba(0,0,0,.11)` / `rgba(255,255,255,.13)` | opaque #d1d1d6 / #343436; split divider gray4 #d1d1d6 / #3c3c3e | tokens.css `--separator` |
| 17 | System colours | `--sys-blue #007aff/#0a84ff`, `--sys-red #ff3b30/#ff453a`, `--sys-orange #ff9500/#ff9f0a`, green #34c759 | blue **#0071e3 / #009aff**, red **#e30000 / #ff3037**, orange #ff9500 / #ff9f0a, green **#03a10e / #32d158** | reminders.css `:root --sys-*`; reminders.css `.rx-switch` green fallback |
| 18 | Destructive red | `--danger #c1121f` / `#ff6b6b` | systemRed #e30000 / #ff3037 | tokens.css `--danger` |
| 19 | Primary button | `.primary` padding 5px 14px, radius 7, 13px/550, background `--gold-deep` | block primary 30px tall, padding 16, radius 8, 14px, appTint background, white | app.css `.primary` (colour **DELIBERATE**, app.css:89–94; the size and radius are not) |
| 20 | Secondary button | `.quiet` 0.5px border, field background, 4px 10px, radius 7 | borderless push (30px, tint text, hover fillTertiary) or block secondary (gray5 fill, no border) | app.css `.quiet`, `.chev`, `.today-btn`, `.cx-today`, `.picker-button` |
| 21 | Segmented control | `.segmented` padding 2, gap 2, radius 7, track text @7%; `.segment` 11px/510, 3px 10px, radius 5; selected 590 with a 0 1px 2px shadow | track fillTertiary, radius 8; segment **24px**, min-width 78, padding 0 16, radius 6, **14px**; selected 600, 0 3px 8px fillTertiary, background **gray2 #636366 in dark** | app.css `.segmented`, `.segment`, `.segment[aria-pressed]` |
| 22 | Toggle | `.rx-switch-track` 34×20, knob 16, on = green, 160ms | 28×17, knob 15, **on = appTint**, off gray4, .25s | reminders.css `.rx-switch-track` |
| 23 | Popover | `.cx-popover` radius 12, 1px glass-edge border, 110ms fade | radius 12, .2px fillQuaternary border, shadow 0 11px 34px; opens .2s scale .5→1 with cubic-bezier(.25,.1,.25,1.3) | calendar.css `.cx-popover`, `@keyframes cx-pop` |
| 24 | Modal sheet | `.modal-sheet` radius 12, padding 20px 22px, 160ms scale .98 | radius **11**, padding 20, .2s cubic-bezier(.32,.08,.24,1), scale .9→1 | app.css `.modal-sheet`, `@keyframes modal-sheet-in` |
| 25 | Shadow | `--shadow-sheet 0 14px 44px rgba(0,0,0,.15), 0 1px 3px rgba(0,0,0,.09)` / dark .55 | 0 11px 34px rgba(0,0,0,.16) / dark **.65** | tokens.css `--shadow-sheet` |
| 26 | Search fields | `.notes-search` / `.rx-search` / `.cx-search` 26–28px tall, text @7% fill | text field 36px, 1px gray3 border, radius 8, 14px; focus is an inset 3px tintBlue ring with no border | notes.css `.notes-search`; reminders.css `.rx-search`; calendar.css `.cx-search` (the iCloud search boxes themselves are UNKNOWN) |
| 27 | Titles | `.content h1` 28/600 −0.022em; `.rx-head h1` 26/700; `.notes-list-title h1` 17/700 | Reminders list title 32/700 in the list colour; Drive folder 24/500; Notes nav title 17/600 | app.css `.content h1`; reminders.css `.rx-head h1`, `.rx-head-count` |

### Low visibility
| # | What | Quince now | icloud.com | Where |
|---|---|---|---|---|
| 28 | Letter-spacing | negative tracking on titles (−0.015 to −0.022em) in app.css, notes.css and reminders.css | none; tracking comes from the SF Text/Display font cuts | `.sheet h1`, `.content h1`, `.cal-nav h1`, `.nf-title`, `.rx-head h1`, `.rx-tile-count` |
| 29 | Focus ring | 3px `--gold-ring` outline (offset varies); Reminders and Calendar use sys-blue @40% | inset 3px tintBlue rgba(0,113,227,.7); buttons add a 1px white inner ring | every `:focus-visible` rule |
| 30 | Motion curve | `--ease cubic-bezier(.32,.72,0,1)`; sidebar width 260ms; hover 120–140ms | popups .2s cubic-bezier(.32,.08,.24,1); cards .35s cubic-bezier(0,0,0,1); hover and selection instant | tokens.css `--ease`; app.css `.sidebar`, `.content` transitions |
| 31 | Menu animation | none (matches iCloud: menus are instant) | none | — (already matches) |
| 32 | Disabled | opacities vary (0.4–0.5), or `--text-3` | opacity .3 light / .4 dark | `:disabled` rules |
| 33 | Calendar checkbox | `.cx-cal input` 14px, radius 4, 1.5px border | 16px SVG checkbox with the fill in the calendar colour and a white or dark check chosen by contrast | calendar.css `.cx-cal input` |
| 34 | Notes checklist circle | `.nf-circle` 17px, 1.5px text-3 border, gold fill | Notes uses the yellow key colour (rendered by the canvas editor; size UNKNOWN) | notes.css `.nf-circle` |
| 35 | Toolbar | none (controls sit inline in the content) | 44px app toolbar, backgroundLayer1, 1px gray5 bottom border, 36px icon buttons | structural; no single selector |

## icloud.com motion, exact (VERIFIED-IN-CODE, shared ui-kit CSS in design/raw/*/*.css, 2026-10-19)
| Element | Animation |
|---|---|
| Popover opening (`ui-pane.is-opening ui-popover.standard`) | `fadeInAndScale` 200ms `cubic-bezier(.25,.1,.25,1.3)`: opacity 0→1, scale .5→1 (slight overshoot) |
| Pop-up menu opening (`ui-pane.for-ui-popup.standard.is-opening ui-popup`) | `fadeInAndBarelyScale` 200ms `cubic-bezier(.32,.08,.24,1)`: opacity 0→1, scale .9→1 |
| Select menu closing (`.ui-select-button-menu`) | `fadeOut` 200ms ease, forwards |
| Menu item chosen with the mouse | `uiMenu-standard-mouse-keyframes-flash`: a brief `--theme-color-fillTertiary` flash before close |
| Modal dialog | `cw-modal-fade-in` (backdrop) + `cw-modal-pop`: opacity 0→1 by 80%, scale .5→1 |
| Bottom sheets / toasts | `slideIn` / `slideOut`: translateY(100%)↔0 |
Every one animates **only opacity and transform**, and none of these surfaces uses `backdrop-filter`:
`.cw-modal-dialog` is an opaque `#fff` with a box-shadow. No sidebar width transition was found in the shared CSS; the
sidebar collapse is not animated in this kit.

**For Quince (WebKitGTK):** the flicker came from blur and from animating layout (width, padding). Quince now has no
blur on these surfaces, so it could bring back icloud.com's exact 200ms opacity+transform open for popovers and menus
without the old glitch. Keep the sidebar fold as a single step, which matches icloud.com. Try it in the real GTK window
first, because his machine is the judge.
