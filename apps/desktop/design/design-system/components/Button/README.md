# Button

Labelled push button in four variants: secondary (default), primary, danger and plain.

- **Primary** at most once per view, for the thing the view is for ("Send 3 Captures", "Install 3 on Unit"). In sheets it sits bottom-right, last.
- **Danger** only on the button that actually erases or removes ("Erase and Create", "Remove"). Entry points that open a confirmation use a secondary button with `destructiveText` ("Remove from Unit…").
- **Plain** for low-weight actions in toolbars and banners ("Show Log", "Change…", "Dismiss").
- Sizes: `sm` (24 px) in tables, banners and the gain presets; `md` (28 px) default; `lg` (34 px) for the one big action on a page ("Create SD Card…").
- Label = verb + object, Title Case. Add `…` when another step follows.
- When a running operation blocks a button, keep it visible and disabled and say why next to it or in its `title`: "Available when the transfer finishes".
- `icon` adds a 14 px icon before the label. With `iconOnly`, a text child ("Refresh") becomes the accessible name and tooltip instead of showing.
