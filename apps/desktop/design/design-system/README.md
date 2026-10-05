# TMP NAM

The design system of TMP NAM, a macOS companion app for the Fender Tone Master Pro booted from a NAM SD card. It manages the NAM captures on the unit, installs tones from Tone3000 and builds the bootable card. The person using it is a guitarist, not necessarily technical, who opens it about once a week.

The look is a plain, native Mac tool in **dark appearance only**. There is no light theme: the window stays dark whatever the system setting.

## Principles

- **The unit is the subject.** Its connection state is always visible at the bottom of the sidebar. There is no Connect button; detection is automatic, and every page that needs the unit says how to get it connected.
- **Say what happens.** Buttons name the action and the object: "Send 3 Captures", "Erase and Create", "Install 3 on Unit". Never "OK", "Continue" or "Submit" outside system dialogs.
- **Long work never traps you.** Sending files, installing tones and building a card show progress in the page *and* in the sidebar's activity card, so you can look around. Actions that would conflict are disabled with a one-line reason; nothing is hidden.
- **Changes that apply later say so.** Size and gain are saved to the unit but heard the next time the capture is selected. Use the `note` banner for this every time.

## Voice and copy

Plain, short, specific. Write for a guitarist, not a developer.

| Do | Don't |
| --- | --- |
| "Connect the Tone Master Pro" | "Device not detected" |
| "2 of 3 captures added. Rectifier Modern Red didn't load after the engine restart." | "Operation partially failed." |
| "Size labels don't guarantee a capture runs without glitches on the unit. Check each one by ear." | "Performance may vary." |
| "Erases Generic STORAGE DEVICE. Takes several minutes." | "Are you sure? This action cannot be undone!" |

- Title case for buttons and menu items ("Add Captures…"); sentence case for everything else.
- An ellipsis on a button means another step follows (a picker, a sheet, a password prompt).
- Name gear as Make · Model · Type. Missing values are an em dash.
- Gain is written as a multiplier (`4×`) in lists and with its dB value in the inspector (`+12.0 dB`).
- Errors say what happened, what it affects, and what to do next. No apologies.

## Colour

Neutral greys with a slight cool bias. One blue for action and selection. Green, amber and red only mean status and always come with text.

- Surfaces: `bg-content` for panes and lists, `bg-subtle` for the inspector, footers and progress panels, `bg-sidebar` for the source list, `bg-control` for controls. `bg-window` sits behind sheets.
- Text: `text` (15.2:1), `text-2` for descriptions, `text-3` for captions and column headers (7.0:1, 4.7:1 on `select`). `text-disabled` is for disabled labels only.
- Action: `accent` fills the one primary button, the slider, the selected size and progress bars. `accent-text` for links and plain buttons. `select` for the selected table row.
- Status: `ok-dot` connected/verified/done, `warn-dot` busy or needs attention, `danger` error text, `danger-fill` the button that erases or removes. Tags pair a `*-weak` ground with the matching text token.
- Never use colour alone to carry meaning; every dot has a label next to it.

## Type

The system font (SF Pro) at AppKit sizes; SF Mono for hashes, file names, commands and logs. Classes come from tokens.css: `t-display` (outcome screens only), `t-title`, `t-toolbar`, `t-head`, `t-body`, `t-small`, `t-caption`, `t-label` (add `text-transform: uppercase`), `t-mono`. Numbers that change or line up use `font-variant-numeric: tabular-nums`.

## Layout

- Window 960 × 700 by default, minimum 720 × 520. Below 820 px wide the sidebar collapses to icons (`Sidebar compact`) and the inspector becomes a sheet.
- Sidebar 200 px · toolbar 52 px · capture inspector 300 px.
- Page padding 20 × 24; inspector sections 16 × 18; controls 8 apart. Radii: `radius-tag` 4, `radius-control` 6, `radius-card` 10, `radius-window` 12.
- Sheets drop from the toolbar edge (`Sheet`) and block only the window. Long operations never run inside a sheet.
- Tables: 44 px rows, name with a caption line under it, flags as tags in that caption line.
- Sizes: an A2 file is a container of several sizes, switchable on the unit (`SizePicker`). An A1 file is one size (Standard, Lite, Feather or Nano) fixed at download; show it in the tag when known (`A1 · Feather`) and point to Tone3000 for other sizes.

## Iconography

16 px, 1.5 px stroke, round caps, drawn on a 16 grid, colour from the text. Icons appear only where they name a place (the sidebar) or a state (banners, stage list, the USB empty state). No decorative icons, no illustrations, no emoji. The set is in the `Icon` component: captures, tone3000, sdcard, settings, warning, error, info, usb, add, minus, check, close, external, bookmark, refresh, lock, chevron.

There is no logo yet; the app name is set in plain type.

## Using it

- Load, in order: `tokens.css`, `components/bundle.css`, then React 18 and `components/bundle.js`. The bundle assigns `window.TmpNam`.
- Put the class `tn-root` on the element that holds the UI (sets the font, text colour and dark `color-scheme`).
- Screens and layout are your own markup styled with the tokens (`var(--bg-content)`, `var(--sep)`, `var(--space-4)`…). Controls and recurring pieces are components: `TmpNam.Button`, `Sidebar`, `Toolbar`, `Banner`, `ProgressBar`, `StageList`, `Sheet`, `SizePicker`, `GainControl`, `Tag`, `TextField`, `Checkbox`, `Radio`, `PopupButton`, `Menu`, `SegmentedControl`, `StatusDot`, `Spinner`, `UnitStatus`, `ActivityCard`, `ConnectSteps`, `Icon`.
- Most components work controlled (`value` + `onChange`) or uncontrolled. `Sheet` renders an absolutely positioned scrim, so its parent must be `position: relative` (the window).
