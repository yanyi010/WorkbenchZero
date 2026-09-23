// @ts-check
import eslint from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  {
    ignores: [
      '**/dist/**',
      '**/node_modules/**',
      'target/**',
      'apps/desktop/src-tauri/resources/**',
      'apps/desktop/dist/**',
      '**/*.min.js',
    ],
  },
  eslint.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,
  {
    languageOptions: {
      parserOptions: {
        projectService: {
          allowDefaultProject: ['*.mjs', 'scripts/*.mjs', 'packages/devtools/src/*.mjs'],
        },
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // zustand's `useApp.getState().fn()` access pattern trips this rule
      // (it targets detached class methods); harmless here.
      '@typescript-eslint/unbound-method': 'off',
      // Deliberate no-op handlers (`() => {}` for catch/disposers).
      '@typescript-eslint/no-empty-function': [
        'error',
        { allow: ['arrowFunctions', 'functions', 'methods'] },
      ],
      // The hyperscript SDK intentionally returns HTMLElement; call sites
      // cast when they need .value etc.
      '@typescript-eslint/no-unnecessary-type-assertion': 'warn',
      // Tests use `any` for wire fixtures deliberately.
      '@typescript-eslint/no-explicit-any': 'off',
      '@typescript-eslint/no-unsafe-assignment': 'warn',
      '@typescript-eslint/no-unsafe-member-access': 'warn',
      '@typescript-eslint/no-unsafe-call': 'warn',
      '@typescript-eslint/no-unsafe-argument': 'warn',
      '@typescript-eslint/no-unsafe-return': 'warn',
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
      // Plugins/plugins use console.* for CLI-style output.
      'no-console': 'off',
      // Ergonomics we accept in UI code.
      '@typescript-eslint/non-nullable-type-assertion-style': 'off',
      '@typescript-eslint/prefer-nullish-coalescing': 'off',
    },
  },
  {
    files: ['**/*.test.{ts,tsx}', 'tests/**'],
    rules: {
      '@typescript-eslint/no-non-null-assertion': 'off',
      '@typescript-eslint/require-await': 'off',
    },
  },
  {
    files: ['packages/devtools/**/*.mjs', 'scripts/**/*.mjs', 'eslint.config.mjs'],
    languageOptions: {
      globals: { ...globals.node },
    },
    rules: {
      // CLI scripts have process.exit flows that trip these.
      '@typescript-eslint/no-unsafe-member-access': 'off',
      '@typescript-eslint/no-unsafe-call': 'off',
    },
  },
);
