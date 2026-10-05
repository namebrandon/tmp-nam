# Icon

The app's 17 stroke icons, drawn on a 16 px grid with a 1.5 px stroke in the current text colour.

- Use only where an icon names a place (sidebar sections) or a state (banners, stages, the USB empty state). Never decorate headings or list items.
- Names: captures, tone3000, sdcard, settings, warning, error, info, usb, add, minus, check, close, external, bookmark, refresh, lock, chevron.
- Decorative by default (`aria-hidden`). Pass `aria-label` when the icon is the only thing conveying meaning.
- `external` marks an action that opens the browser ("Sign In with Browser").
