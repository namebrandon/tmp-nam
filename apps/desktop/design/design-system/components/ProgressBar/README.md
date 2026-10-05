# ProgressBar

A 6 px bar for determinate or indeterminate progress, with an optional label and right-aligned detail.

- Determinate (`value` 0–100) for anything with a size: sending files, downloading tones, writing the card. Put the real quantity in `detail` ("143 of 298 KB"), not just a percentage.
- Indeterminate for the engine restart and other short waits of unknown length.
- `tone="ok"` when finished and verified; `tone="danger"` frozen at the point of failure.
- `size="lg"` (8 px) once per page, for the overall progress of the SD card build.
