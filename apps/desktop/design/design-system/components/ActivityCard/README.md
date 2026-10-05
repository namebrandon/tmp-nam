# ActivityCard

The sidebar card for a running long operation: title, one detail line and a progress bar.

- One at a time; the app never starts a conflicting operation.
- Detail names the current item and the count ("2 of 3 · JC-120 Clean") or the time left.
- Usually passed to `Sidebar` as `activity`, not mounted alone.
