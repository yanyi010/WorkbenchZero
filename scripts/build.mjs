/**
 * Workbench Zero plugin build (spec §38, §41, §113).
 *
 * For each plugin under plugins/ and examples/:
 *   1. bundle src/main.ts → dist/main.js (+ dist/main.css when the bundle
 *      pulls in CSS) with esbuild,
 *   2. copy plugin.json, entry.html, style.css and the shared design
 *      tokens into the package,
 *   3. collect the manifest into the registry.
 *
 * Output layout (resources/, bundled by tauri):
 *   resources/registry/index.json    — CatalogEntry[] (bare array)
 *   resources/packs.json             — ExtensionPack[] (bare array)
 *   resources/plugins/<id>/…         — installable plugin packages
 *
 * `--watch` recompiles on change for `wb dev`-style workflows.
 */
import { build, context } from 'esbuild';
import { cp, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
const resources = path.join(root, 'apps/desktop/src-tauri/resources');
const watch = process.argv.includes('--watch');

const PACKS = [
  {
    id: 'essentials',
    name: 'Essentials',
    description: 'Memo, Tasks, Sticky and Quick Ask — the daily capture kit.',
    plugins: ['zero.memo', 'zero.tasks', 'zero.sticky', 'zero.ai'],
  },
  {
    id: 'developer',
    name: 'Developer',
    description: 'Terminal and Files for technical workspaces.',
    plugins: ['zero.files', 'zero.terminal'],
  },
  {
    id: 'minimal',
    name: 'Minimal',
    description: 'Just memo capture. Nothing else.',
    plugins: ['zero.memo'],
  },
];

async function pluginDirs() {
  const out = [];
  for (const base of ['plugins', 'examples']) {
    const dir = path.join(root, base);
    if (!existsSync(dir)) continue;
    for (const entry of await readdir(dir)) {
      const manifest = path.join(dir, entry, 'plugin.json');
      if (existsSync(manifest)) out.push(path.join(dir, entry));
    }
  }
  return out;
}

async function buildPlugin(dir, { dev }) {
  const manifest = JSON.parse(await readFile(path.join(dir, 'plugin.json'), 'utf8'));
  const id = manifest.id;
  const dist = path.join(dir, 'dist');
  await mkdir(dist, { recursive: true });

  const options = {
    entryPoints: [path.join(dir, 'src/main.ts')],
    bundle: true,
    format: 'esm',
    target: ['chrome120'],
    outfile: path.join(dist, 'main.js'),
    sourcemap: dev ? 'inline' : false,
    minify: !dev,
    logLevel: 'warning',
    legalComments: 'none',
    define: { 'process.env.NODE_ENV': JSON.stringify(dev ? 'development' : 'production') },
  };

  if (watch) {
    const ctx = await context(options);
    await ctx.watch();
    console.log(`[watch] ${id}`);
    return { manifest, watched: true };
  }

  const result = await build({ ...options, write: true });
  if (result.errors.length > 0) throw new Error(`esbuild failed for ${id}`);

  // Package payload: everything a sandboxed iframe needs to run offline.
  const dest = path.join(resources, 'plugins', id);
  await mkdir(dest, { recursive: true });
  await cp(path.join(dir, 'plugin.json'), path.join(dest, 'plugin.json'));
  if (existsSync(path.join(dir, 'entry.html'))) {
    await cp(path.join(dir, 'entry.html'), path.join(dest, 'entry.html'));
  }
  if (existsSync(path.join(dir, 'style.css'))) {
    await cp(path.join(dir, 'style.css'), path.join(dest, 'style.css'));
  }
  await cp(path.join(root, 'packages/ui-kit/src/tokens.css'), path.join(dest, 'tokens.css'));
  await cp(dist, path.join(dest, 'dist'), { recursive: true });

  return { manifest, watched: false };
}

async function main() {
  const dev = process.argv.includes('--dev') || watch;
  await mkdir(path.join(resources, 'plugins'), { recursive: true });
  await mkdir(path.join(resources, 'registry'), { recursive: true });

  const dirs = await pluginDirs();
  const entries = [];
  for (const dir of dirs) {
    const { manifest, watched } = await buildPlugin(dir, { dev });
    if (watched) continue; // watch mode never finalizes the registry
    entries.push({
      id: manifest.id,
      name: manifest.name,
      version: manifest.version,
      publisher: manifest.publisher ?? '',
      description: manifest.description ?? '',
      repository: manifest.repository,
      license: manifest.license,
      packagePath: `plugins/${manifest.id}`,
      categories: manifest.categories ?? [],
    });
  }

  if (!watch) {
    entries.sort((a, b) => a.id.localeCompare(b.id));
    await writeFile(
      path.join(resources, 'registry', 'index.json'),
      JSON.stringify(entries, null, 2) + '\n',
    );
    const packs = PACKS.filter((p) => p.plugins.every((id) => entries.some((e) => e.id === id))).map(
      ({ id, name, description, plugins }) => ({ id, name, description, plugins }),
    );
    await writeFile(path.join(resources, 'packs.json'), JSON.stringify(packs, null, 2) + '\n');
    console.log(`built ${entries.length} plugins → ${path.relative(root, resources)}`);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
