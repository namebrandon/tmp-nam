# SizePicker

Chooses which size of an A2 capture the player loads, ordered smallest to largest.

- Labels come from the container itself; the first segment is captioned "smallest", the last "largest".
- Only for A2 captures: an A2 file contains several sizes and the player switches between them. An A1 file contains exactly one size (Standard, Lite, Feather or Nano), chosen when it was downloaded, so there is nothing to switch on the unit. For A1 the inspector names the size when it's known ("This file is the Feather size. An A1 file holds only one size, so to try another, install that variant from Tone3000.") with a Show in Tone3000 button, or says "An A1 file holds only one size. To use another size, add that variant's .nam file." when it isn't.
- Under it: "Larger sizes sound closer to the amp but use more of the unit's processing. If it crackles, go smaller." After any change, show the `note` Banner: applies on reselect.
