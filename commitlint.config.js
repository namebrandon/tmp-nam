export default {
  extends: ['@commitlint/config-conventional'],
  // Dependabot's commit body (release notes, compare links) exceeds body-max-line-length;
  // its subject is already a Conventional Commit. Only a sign-off trailer line in the body
  // counts, so a one-line PR title that merely mentions it is still linted.
  ignores: [(commit) => /\n\s*Signed-off-by: dependabot\[bot\]/.test(commit)],
};
