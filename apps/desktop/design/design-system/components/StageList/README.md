# StageList

An ordered list of steps in a long operation, each done, current, pending or failed.

- The SD card build lists its eight stages: checking the firmware file, extracting, verifying, adding NAM support, building the filesystem, preparing partitions, writing, reading back to verify.
- The current stage shows a spinner and a time hint in the detail column; the slow stages (writing, verifying) say "minutes".
- On failure, the failed stage turns red and later stages are left out.
