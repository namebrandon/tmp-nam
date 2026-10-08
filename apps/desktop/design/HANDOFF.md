# TMP NAM — design handoff

TMP NAM is a macOS companion app for the Fender Tone Master Pro booted from a custom NAM SD card. It manages the NAM captures on the unit, installs tones from Tone3000, and builds the bootable SD card. The user is a guitarist, not necessarily technical, who opens it about once a week.

This folder is everything needed to build the frontend. Read this file first, then `design-system/README.md`.

## What's in this folder

| Path | What it is | Use it for |
| --- | --- | --- |
| `HANDOFF.md` | This spec | Structure, states, behaviour, open questions |
| `design-system/README.md` | The brand book | Voice, colour rules, type, layout, iconography |
| `design-system/tokens.json` | Colours, type styles, spacing, radii, shadows, sizes | Source of truth for every value |
| `design-system/components/<Name>/README.md` | Guidelines per component | When and how to use each piece |
| `design-system/components/<Name>/preview.html` | Live usage examples | Prop usage; open in the published design system to see them render |
| `design-system/components/bundle.js`, `bundle.css`, `index.d.ts` | React 18 implementation of the 22 components (`window.TmpNam`) | Reuse directly in a web stack; read as reference in a native one |
| `prototype/Main.dc.html` | Source of the clickable prototype | Flows, state transitions, edge cases, timings, exact copy |
| `screens/*.dc.html` + `screens/screens.css` | Source of the 38 static screens | Exact layout and copy of every state |

The `.dc.html` files are design-canvas documents: HTML plus `{{ }}` template holes and a small logic class. Read them for structure, copy and behaviour; don't run them as-is.

Published versions (viewable in a browser, same account):
- Design system: https://claude.ai/artifact/SzVNMkG71aEqPu9srbQTZn
- Static screens: https://claude.ai/artifact/Futxy4oA6ksyPPYFsnsoaK
- Prototype: https://claude.ai/artifact/Gvb7J6kbSJ8L4GyPveEE31

## Stack note

- **Web-based Mac app (Tauri, Electron, WebView):** load `tokens` as CSS variables (one `--<name>` per token; values in `tokens.json`), then `bundle.css` and `bundle.js` after React 18. Components are classic React; port them to your module system or keep the global.
- **Native (SwiftUI/AppKit):** use `tokens.json` for colours (asset catalog, dark appearance only), type, spacing and radii, and the component READMEs as behaviour specs. Prefer native controls where they match (NSTableView/`Table`, sheets, `Picker(.segmented)`, `ProgressView`, `SecureField`, `Stepper` + `Slider`). The bundle is reference only.

## Global rules

1. **Dark appearance only.** Force it regardless of the system setting.
2. **Window:** default 960 × 700, minimum 720 × 520. Below 820 px wide the sidebar collapses to icons (64 px) and the capture inspector becomes a sheet opened by a "Size & Gain…" toolbar button.
3. **No Connect button.** The unit is detected automatically over USB. Its state is always visible at the bottom of the sidebar: connected ("Connected · NAM card"), looking ("Looking for the unit…"), not connected, busy ("Engine restarting").
4. **Pages that need the unit** show the connect steps when it's missing: insert the NAM SD card and power on → wait for the preset screen → plug in USB-C. "It's detected automatically."
5. **One long operation at a time:** sending captures, installing tones, building the SD card. While one runs:
   - it shows in its page *and* in the sidebar activity card (title, detail, progress), visible on every page;
   - navigation stays enabled;
   - every action that would start a conflicting operation stays visible but disabled, with the reason in a tooltip or next to it ("Available when the transfer finishes");
   - editing size, gain and removal pauses during USB transfers.
6. **Results of long operations** appear as a banner at the top of the page, never as a modal.
7. **Sheets** drop from the toolbar edge and block only this window. They're used for: checking files before sending, confirming removal, allowed variants, choosing the SD card's firmware, confirming the card erase. No long operation runs inside a sheet.
8. **Copy is final.** It's in the screens and prototype. Title Case on buttons and menu items, sentence case elsewhere, `…` when another step follows.

## Information architecture

Sidebar: **Captures** (with count) · **Tone3000** · **SD Card** · (bottom) **Settings**, then the activity card when something runs, then the unit status.

### Captures

Toolbar: title, subtitle "N on unit · size", primary **Add Captures…**.

Body: a table (Name with a gear caption line "Make · Model · Type", Type tag, file Size) plus a 300 px inspector for the selected row. Footer: "N captures need attention · select one to fix it" when flags exist.

Type tag:
- A2 → `A2 · <number of sizes>` (inspector: `A2 · 3 sizes`)
- A1 → `A1 · <size>` when the size is known (installed from Tone3000), else `A1`

Inspector, normal capture:
- Header: name, gear line, tags (architecture, sample rate, file size).
- **Size** (A2 only): segmented picker, smallest → largest, labels taken from the container ("Feather … Full"; some containers have 3+ sizes). Help: "Larger sizes sound closer to the amp but use more of the unit's processing. If it crackles, go smaller."
- **Size** (A1): "This file is the Feather size. An A1 file holds only one size, so to try another, install that variant from Tone3000." + **Show in Tone3000** (opens the tone with its model menu). When the size is unknown: "An A1 file holds only one size. To use another size, add that variant's .nam file."
- Output gain is configured on the unit, not edited in the desktop. A size change preserves the gain read from the unit when saving, including changes made since the last list refresh. Full removes only the size override.
- After a size change: blue note "Saved to the unit. The size applies the next time you select this capture on the unit. If it's selected now, pick another capture and come back." The row shows a blue dot.
- If player settings cannot be read or listed options are invalid, Captures shows **Player settings unavailable** with **Refresh settings**. No A2 size is selected until a reliable refresh. An absent settings file uses defaults without a warning. A size change may recover malformed settings with a backup and a separate **Player settings recovered** warning; see [nam-player.md](../../../docs/nam-player.md).
- **Remove from Unit…** → sheet: "Remove “<name>” from the unit?" / "Presets that use this capture will lose it. On the unit, you'll need to pick another IR in those presets." / Cancel (default focus) · **Remove** (red).

Flags:
- **File missing** (registered, file gone): error banner; "Expected file <name>.nam"; **Choose File…** to restore, **Remove Entry…**.
- **Not registered** (file present, not listed): warn banner; **Register**, **Delete File…**.
- **Didn't load** (see below): **Send Again**, **Remove…**.

States: looking for the unit · not found (connect steps + "No NAM SD card yet? Make an SD Card") · connected with no captures (empty state: Add .nam Files… / Install from Tone3000; drop onto window) · list.

**Add flow:**
1. File picker for one or more `.nam` files.
2. Sheet "Add captures to the unit": each file checked. Valid ones show gear, architecture, rate, size; invalid ones show "Not a valid .nam capture. The file is damaged or incomplete." and are skipped. Note: captures show up in the unit's IR list as soon as they're sent; if Pro Control is open, the audio engine restarts instead and is silent for a few seconds. Buttons: Add More Files… (left) · Cancel · **Send N Captures**.
3. Sending panel above the table: overall progress plus per file (Sent · size / "143 of 298 KB" / Waiting). "Keep the USB cable connected. You can switch pages; the transfer keeps going." Sidebar activity "Sending captures · 2 of 3 · <name>".
4. Engine restart (only when the HID channel is unavailable, e.g. Pro Control open): indeterminate progress; unit status "Engine restarting".
5. Result banner:
   - "3 captures added": new rows carry a "New" tag and start at 1×.
   - Or "2 of 3 captures added. <name> was sent, but the unit didn't load it after the engine restart.": Send Again / Remove It.

**Errors:**
- **Unit disconnects mid-transfer:** "The unit disconnected while sending. 1 of 3 captures reached the unit. The interrupted file was not kept. Reconnect to send the rest." with **Send Remaining N** (enabled once reconnected) and Discard. The list of what was sent, interrupted and not sent is kept.
- **Disconnect during the engine restart:** "All N captures were sent. Reconnect to check they loaded."

### Tone3000

Account states:
1. **No API key:** explain, two steps (create a key on tone3000.com › Settings › API; paste it), field + **Save Key**. "Stored in your Mac's Keychain."
2. **Key saved, signed out:** **Sign In with Browser** (opens the system browser).
3. **Signing in:** "Waiting for you in the browser", Open Page Again, Cancel.
4. **Error:** "Tone3000 didn't accept your API key", field in error, Save and Sign In. Same slot for declined, network and timeout errors.
5. **Loading:** spinner, "Loading your tones".
6. **Signed in:** "Signed in as <username>" in the toolbar subtitle.

List (signed in):
- Toolbar: segmented All · Bookmarked · Mine, Refresh.
- Strip: "Picking from your allowed variants: A2 Feather, Nano · A1 Feather, Nano" + Change… (opens the Allowed variants sheet).
- Columns: checkbox · Tone (title, "by <author>", "only A1 published" where relevant) · Source tag (Bookmark / Yours) · Model pop-up · On unit (tag "On unit" / "Didn't load" / progress text / "Unknown" when the unit isn't connected).
- **Model pop-up menu:** "Allowed by your settings" (checked = current; the automatic one is noted "picked automatically"), separator, "Published, not allowed" (disabled), separator, "Change Allowed Variants…". A manual override puts a blue dot on the pop-up.
- **No allowed model:** amber pop-up "No allowed model", checkbox disabled.
- **Footer:** "N selected · about X MB to download", Clear, **Install N on Unit**. If the unit isn't connected: "Connect the unit to install. Your selection is kept." and the button is disabled. Browsing never needs the unit.
- **Install:** download (Tone3000) → send over USB (→ engine restart only in the fallback), with a three-step panel and per-row status. Result banner as in Captures; installed tones get "On unit" and appear in Captures (new, 1×). A disconnect offers **Install Remaining N**.
- **Empty:** "No tones yet", Open tone3000.com, Refresh.

**Auto-pick rule:** among the tone's published models that are allowed, prefer A2 over A1, then the largest allowed size. Size order: A2 Full > Feather > Nano; A1 Standard > Lite > Feather > Nano.

**Allowed variants** (sheet here, and a section in Settings, same setting):
- A2: Full, Feather, Nano. A1: Standard, Lite, Feather, Nano.
- Default: A2 Feather + Nano, A1 Feather + Nano.
- Text: "When a tone has more than one allowed model, the sync picks A2 first, then the largest allowed size. You can still choose another model per tone."
- Warning: "Size labels are not a guarantee. A model labelled Feather or Nano can still crackle or drop out on the unit. Check every capture by ear before using it live."
- Buttons: Restore Default · Cancel · Save. Saving clears overrides that are no longer allowed.

### SD Card

Two inputs on one page plus an "About the card" aside:
- The unit's internal firmware is never modified.
- Removing the card boots stock again.
- Keep your current working card as a spare.

1. **Fender firmware file:** Choose File… → `ToneMasterPro_v1_8_58.img` checked against the known hash. Results: tag **Verified**, or **Wrong file**: "This isn't the expected firmware. The card needs exactly ToneMasterPro_v1_8_58.img, unmodified. A renamed or partly downloaded file fails this check too."
2. **SD card:** removable disks detected automatically, as a radio list (name, size, bus). Refused disks stay visible, disabled, with the reason ("Not a USB reader"). None: "Insert the SD card into a USB card reader…".

**Prerequisites** (shown above, and they block the page):
- Missing command-line tools, with the Homebrew command, Copy and Check Again.
- Damaged app assets: "Download TMP NAM again and replace this copy."

**Create SD Card…** (enabled when both inputs are valid):
1. Confirm sheet "Erase this card and create the NAM card?" showing name, size, bus and `/dev/diskN`, then "Everything on this card will be deleted…". Cancel (default) · **Erase and Create** (red).
2. macOS administrator password (system dialog).
3. Progress page with an overall bar and stage list:
   1. Checking the firmware file
   2. Extracting
   3. Verifying the extracted files
   4. Adding NAM support
   5. Building the filesystem
   6. Preparing partitions
   7. Writing to the card (minutes, shows % and time left)
   8. Reading back to verify (minutes)

   "Keep the card in and the Mac awake." Show Log / Hide Log (raw log, monospace).
4. Outcomes:
   - **Success:** "Card ready. Written and verified. The card has been ejected." Next: power off → insert the card → power on. Done / Make Another.
   - **Failure:** "The card couldn't be created", the failing stage in red, the message, log open, Try Again / Back. "Your unit and its firmware are unaffected."
   - **Admin denied:** back to the form with "Administrator access wasn't given. Nothing was written to the card…", button **Try Again…**.
   - **Removable Volumes blocked (macOS):** back to the form with "macOS blocked access to the SD card", how to turn on Removable Volumes for TMP NAM in Privacy & Security › Files and Folders, button **Open Privacy & Security** (opens that pane).

### Settings

Segmented: Tone3000 · Allowed Variants · Unit.

- **Tone3000:** masked API key with Replace… / Remove ("Removing it also signs you out"); account status with Sign Out / Sign In….
- **Allowed Variants:** the same setting as the sheet, applied live.
- **Unit:** connection status; SD card build ID and NAM player hash (monospace, Copy each, Copy All Details). Not connected: connect steps, values "Shown when the unit is connected".

## Data the UI needs

- **Unit:** `state` (looking | connected | missing | busy), `cardBuildId`, `namPlayerHash`.
- **Capture:**
  - `id`, `name`, `gear {make, model, type}?`, `sampleRate` (44.1 | 48 kHz), `fileSize`
  - `architecture` (A1 | A2), `sizes[]` (A2, smallest → largest), `selectedSize` (A2; unknown when settings cannot be read reliably)
  - `a1Size?` (A1, when known); gain is preserved on the unit, not edited in the UI
  - `flags` (registeredButMissing | presentButUnregistered | failedAfterRestart), `source` (file | tone3000 + toneId)
- **Capture list:** optional `settings_error`, independent of whether captures are present.
- **Tone:** `id`, `title`, `author`, `source` (bookmark | own), `models[]` ("A2 Feather"…), `override?`, `onUnit`.
- **AllowedVariants:** `{A2: [...], A1: [...]}`.
- **Operation** (singleton): kind (send | install | sdBuild), phase, items with per-item state and bytes, overall progress.
- **SD build:** firmware (none | verified | wrong), selected disk, prerequisites (missing tools[], assets ok), stage, write %, log lines, outcome.

## Open questions and assumptions

These are design assumptions, not known facts. Confirm them while building:

1. **A1 size:** shown only when known. Assumed known only for captures installed from Tone3000. If `.nam` metadata includes it, show it for local files too.
2. **Command-line tools:** the screen names `mke2fs` / `debugfs` and `brew install e2fsprogs` as placeholders. Use the real list.
3. **API key storage:** the Keychain is assumed.
4. **Interrupted transfer:** the copy says the partial file isn't kept. The app must guarantee it, or the copy changes.
5. **Not registered → Register:** the Register action is a design assumption. Confirm the unit supports it.
6. **Gain defaults to 1×** when the model hash has no stored override; adding a capture or changing its size does not reset an existing override.
7. **Refused disks:** only "Not a USB reader" was specified. Other refusal reasons need copy.
8. **Stopping a card build** mid-write isn't designed (the static screen shows a Stop… button, the prototype leaves it out). Decide whether it's allowed.
9. **Sample data:** all names, sizes, hashes, IDs and usernames are made up.

## About the prototype

- The right-hand "Prototype controls" panel is not part of the app. It simulates plugging and unplugging the unit and forcing failures.
- The firmware picker in the prototype is a two-option stand-in for the macOS file dialog; the password prompt imitates the system one.
- Timings (send speed, restart ~2.5 s, card stages) are for demonstration only.
