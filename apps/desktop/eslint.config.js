import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import react from "eslint-plugin-react";
import eslintConfigPrettier from "eslint-config-prettier";

export default tseslint.config(
  {
    ignores: [
      "dist/",
      "src-tauri/",
      "node_modules/",
    ],
  },

  tseslint.configs.eslintRecommended,
  // Production-grade, type-aware linting: the STRICTEST typescript-eslint presets.
  // strict-type-checked is a superset of recommended-type-checked (no-non-null-
  // assertion, no-unnecessary-condition, no-confusing-void-expression, …);
  // stylistic-type-checked adds consistency rules. Both require the parser to load
  // type information — see parserOptions.projectService below.
  ...tseslint.configs.strictTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,
  react.configs.flat.recommended,
  react.configs.flat["jsx-runtime"],

  {
    files: ["src/**/*.{ts,tsx}"],
    // No escape hatches in src/. `any` and non-null `!` are already errors via
    // strictTypeChecked; these two close the remaining holes so the rule is
    // enforced by the linter rather than by prose someone has to have read:
    //   - noInlineConfig makes an `eslint-disable` comment unable to silence
    //     anything, and reportUnusedDisableDirectives then flags it as an error.
    //   - ban-ts-comment below rejects @ts-expect-error too, which the preset
    //     default would otherwise allow when it carries a description.
    // Fix findings by changing code, never by silencing.
    linterOptions: {
      noInlineConfig: true,
      reportUnusedDisableDirectives: "error",
    },
    languageOptions: {
      ecmaVersion: "latest",
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    settings: {
      react: { version: "detect" },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs["recommended-latest"].rules,
      // Superseded by TypeScript's own type system — props are validated by the
      // compiler, so the runtime prop-types check is redundant (not silenced).
      // (react-in-jsx-scope / jsx-uses-react are likewise off via jsx-runtime
      // above, as the React 19 automatic JSX transform requires.)
      "react/prop-types": "off",
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // The preset default permits @ts-expect-error when it has a description;
      // this repo permits none of the three.
      "@typescript-eslint/ban-ts-comment": [
        "error",
        {
          "ts-expect-error": true,
          "ts-ignore": true,
          "ts-nocheck": true,
          "ts-check": false,
        },
      ],
    },
  },

  {
    files: ["*.config.{js,ts}", "vite.config.ts", "vitest.config.ts"],
    languageOptions: {
      globals: globals.node,
    },
  },
  // Root-level config files aren't part of the tsconfig `include` (src/e2e only), so they
  // have no type info for the type-checked presets above — drop just those rules, keep the rest.
  {
    files: ["*.config.{js,ts}", "vite.config.ts", "vitest.config.ts"],
    ...tseslint.configs.disableTypeChecked,
  },

  eslintConfigPrettier,
);
