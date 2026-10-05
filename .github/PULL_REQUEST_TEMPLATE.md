<!-- Every commit and the title must be Conventional Commits: feat: / fix: / docs: / chore: / refactor: … (commitlint gates this). -->

## What & why

<!-- What changed and the reason. Link the issue: Closes #123 -->

## How it was tested

<!-- Delete rows that don't apply. -->

- [ ] `scripts/check.sh all`
- [ ] Simulated unit (`TMP_NAM_SIM=1 bun run tauri dev`)
- [ ] Image-file card built (`tmp-sdcard image …`) and audit passed
- [ ] Verified on a real Tone Master Pro, firmware 1.8.58 (boot, audio, capture switching)

## Checklist

- [ ] Conventional-commit title and commits
- [ ] No hand-edited hashes in `device/release.json` (see BUILDING.md › release pins)
- [ ] Device-side Python stays 3.5-compatible (no f-strings); device shell stays BusyBox ash
- [ ] No lint escape hatches in `apps/desktop/src/` (`eslint-disable`, `any`, `@ts-ignore`)
- [ ] Docs updated (README, `docs/`, `CLAUDE.md`) when behavior changed
