export default {
  extends: ['@commitlint/config-conventional'],
  // Dependabot's commit body (release notes, compare links) exceeds body-max-line-length;
  // its subject is already a Conventional Commit.
  ignores: [(commit) => commit.includes('Signed-off-by: dependabot[bot]')],
};
