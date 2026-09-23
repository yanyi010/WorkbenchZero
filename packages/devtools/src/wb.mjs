#!/usr/bin/env node
/**
 * wb — EigenDesk plugin developer CLI (spec §87).
 *
 *   wb plugin create <name>     scaffold a new plugin
 *   wb plugin build [dir]       bundle src/main.ts → dist/
 *   wb plugin dev [dir]         build into the app's dev-plugins dir, then
 *                               rebuild on change (kernel watcher hot-reloads)
 *   wb plugin pack [dir]        deterministic .edplugin.zip
 *   wb plugin validate [dir]    manifest + permission checks
 */
import { build, context } from 'esbuild';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  cp,
  mkdir,
  readFile,
  readdir,
  stat,
  writeFile,
} from 'node:fs/promises';
import { existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../../..');

const KNOWN_PERMISSIONS = new Set([
  'workspace:read',
  'workspace:write',
  'filesystem:read',
  'filesystem:write',
  'network',
  'process:spawn',
  'clipboard:read',
  'clipboard:write',
  'notification',
  'secrets:read',
  'ai:invoke',
  'mcp:connect',
  'system:open',
]);

const MANIFEST_REQUIRED = ['id', 'name', 'version', 'apiVersion', 'permissions', 'activationEvents'];

function fail(message) {
  console.error(`wb: ${message}`);
  process.exit(1);
}

function dataDir() {
  const base = process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share');
  return process.env.EIGENDESK_DATA_DIR || path.join(base, 'eigendesk');
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

async function create(name, opts) {
  if (!name || !/^[a-z0-9][a-z0-9-]*$/.test(name)) {
    fail('plugin name must be lowercase kebab-case (e.g. my-plugin)');
  }
  const dir = path.resolve(opts.dir || name);
  if (existsSync(dir)) fail(`${dir} already exists`);
  const publisher = opts.publisher || 'community';
  const id = `${publisher}.${name}`;
  const className = name.replace(/-([a-z])/g, (_, c) => c.toUpperCase());

  const files = {
    'plugin.json': `${JSON.stringify(
      {
        id,
        name: className,
        version: '0.1.0',
        apiVersion: '1',
        description: 'A new EigenDesk plugin.',
        publisher,
        trust: 'community',
        permissions: [],
        activationEvents: [`onCommand:${id}.hello`],
        contributes: {
          commands: [{ id: `${id}.hello`, title: `${className}: Hello`, category: className, takesArgs: true }],
          views: [],
          settings: [],
          searchProviders: [],
        },
      },
      null,
      2,
    )}\n`,
    'entry.html': `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <link rel="stylesheet" href="./tokens.css" />
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="./dist/main.js"></script>
  </body>
</html>
`,
    'src/main.ts': `import { definePlugin, h, render } from '@eigendesk/plugin-sdk';

definePlugin({
  async activate(ctx) {
    ctx.commands.onCommand((id, args) => {
      if (id !== '${id}.hello') return undefined;
      return \`Hello, \${args ?? 'world'}!\`;
    });

    if (ctx.surface === 'view:${id}.main') {
      render(document.getElementById('app')!, h('h2', {}, '${className}'));
    }
  },
});
`,
    'README.md': `# ${className}

An EigenDesk plugin.

## Develop

\`\`\`bash
wb plugin dev .        # build + hot reload into the running app
\`\`\`

## Package

\`\`\`bash
wb plugin pack .       # → ${id}-0.1.0.edplugin.zip
\`\`\`
`,
  };

  await mkdir(path.join(dir, 'src'), { recursive: true });
  for (const [file, content] of Object.entries(files)) {
    await writeFile(path.join(dir, file), content);
  }
  console.log(`created ${dir} (id: ${id})`);
  console.log('next: wb plugin dev .');
}

async function loadManifest(dir) {
  const manifestPath = path.join(dir, 'plugin.json');
  if (!existsSync(manifestPath)) fail(`no plugin.json in ${dir}`);
  try {
    return JSON.parse(await readFile(manifestPath, 'utf8'));
  } catch (err) {
    fail(`invalid plugin.json: ${err.message}`);
  }
}

async function bundle(dir, { dev }) {
  const manifest = await loadManifest(dir);
  const entry = path.join(dir, 'src/main.ts');
  if (!existsSync(entry)) fail(`no src/main.ts in ${dir}`);
  const outdir = path.join(dir, 'dist');
  await mkdir(outdir, { recursive: true });

  const options = {
    entryPoints: [entry],
    bundle: true,
    format: 'esm',
    target: ['chrome120'],
    outfile: path.join(outdir, 'main.js'),
    sourcemap: dev ? 'inline' : false,
    minify: !dev,
    logLevel: 'warning',
    legalComments: 'none',
    alias: {
      '@eigendesk/plugin-sdk': path.join(root, 'packages/plugin-sdk/src/index.ts'),
      '@eigendesk/plugin-sdk/h': path.join(root, 'packages/plugin-sdk/src/h.ts'),
      '@eigendesk/plugin-sdk/markdown': path.join(root, 'packages/plugin-sdk/src/markdown.ts'),
      '@eigendesk/protocol': path.join(root, 'packages/protocol/src/index.ts'),
      '@eigendesk/ui-kit': path.join(root, 'packages/ui-kit/src/index.tsx'),
    },
  };
  return { manifest, options };
}

async function buildPlugin(dir, { dev = false } = {}) {
  const { manifest, options } = await bundle(dir, { dev });
  const result = await build({ ...options, write: true });
  if (result.errors.length > 0) fail(`esbuild failed for ${manifest.id}`);
  return manifest;
}

async function devPlugin(dir) {
  const { manifest, options } = await bundle(dir, { dev: true });
  const target = path.join(dataDir(), 'dev-plugins', manifest.id);
  const deploy = async () => {
    await build({ ...options, write: true });
    await mkdir(target, { recursive: true });
    for (const file of ['plugin.json', 'entry.html', 'style.css']) {
      const src = path.join(dir, file);
      if (existsSync(src)) await cp(src, path.join(target, file));
    }
    const tokens = path.join(root, 'packages/ui-kit/src/tokens.css');
    if (existsSync(tokens)) await cp(tokens, path.join(target, 'tokens.css'));
    await cp(path.join(dir, 'dist'), path.join(target, 'dist'), { recursive: true });
    console.log(`deployed ${manifest.id} → ${target} (hot reload picks it up)`);
  };
  await deploy();

  const ctx = await context(options);
  await ctx.watch();
  let timer = null;
  // Debounce: esbuild fires per-file; the kernel watcher needs one clean
  // directory snapshot, not ten racing ones.
  const rebuild = () => {
    clearTimeout(timer);
    timer = setTimeout(deploy, 150);
  };
  for (const watchDir of ['src', '.']) {
    const abs = path.resolve(dir, watchDir);
    if (!existsSync(abs)) continue;
    const { watch } = await import('node:fs');
    watch(abs, { recursive: true }, (event, file) => {
      if (file?.startsWith('dist/')) return;
      rebuild();
    });
  }
  console.log('watching for changes… (Ctrl+C to stop)');
}

// ---------------------------------------------------------------------------
// deterministic .edplugin packaging (stored zip, fixed timestamps)
// ---------------------------------------------------------------------------

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[i] = c >>> 0;
  }
  return table;
})();

function crc32(bytes) {
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

async function collectFiles(dir, base = dir, out = []) {
  for (const entry of (await readdir(dir, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
    if (entry.name === 'dist' && base === dir) continue; // dist is included below
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) await collectFiles(full, base, out);
    else out.push({ name: path.relative(base, full).split(path.sep).join('/'), full });
  }
  return out;
}

async function packPlugin(dir) {
  const manifest = await buildPlugin(dir, { dev: false });
  const files = await collectFiles(dir);
  // Include the freshly built dist.
  for (const f of await collectFiles(path.join(dir, 'dist'), dir)) {
    if (!files.some((x) => x.name === f.name)) files.push(f);
  }
  files.sort((a, b) => a.name.localeCompare(b.name));

  const chunks = [];
  const central = [];
  let offset = 0;
  const DOS_TIME = 0; // 00:00:00
  const DOS_DATE = 0x21; // 1980-01-01 — fixed for determinism

  for (const file of files) {
    const data = await readFile(file.full);
    const nameBytes = Buffer.from(file.name, 'utf8');
    const crc = crc32(data);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0, 6); // flags
    local.writeUInt16LE(0, 8); // stored
    local.writeUInt16LE(DOS_TIME, 10);
    local.writeUInt16LE(DOS_DATE, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    local.writeUInt16LE(0, 28);

    chunks.push(local, nameBytes, data);

    const cd = Buffer.alloc(46);
    cd.writeUInt32LE(0x02014b50, 0);
    cd.writeUInt16LE(20, 4);
    cd.writeUInt16LE(20, 6);
    cd.writeUInt16LE(0, 8);
    cd.writeUInt16LE(0, 10);
    cd.writeUInt16LE(DOS_TIME, 12);
    cd.writeUInt16LE(DOS_DATE, 14);
    cd.writeUInt32LE(crc, 16);
    cd.writeUInt32LE(data.length, 20);
    cd.writeUInt32LE(data.length, 24);
    cd.writeUInt16LE(nameBytes.length, 28);
    cd.writeUInt16LE(0, 30);
    cd.writeUInt16LE(0, 32);
    cd.writeUInt16LE(0, 34);
    cd.writeUInt16LE(0, 36);
    cd.writeUInt32LE(0, 38);
    cd.writeUInt32LE(offset, 42);
    central.push(cd, nameBytes);

    offset += local.length + nameBytes.length + data.length;
  }

  const centralBuf = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(0, 4);
  end.writeUInt16LE(0, 6);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(centralBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20);

  const zip = Buffer.concat([...chunks, centralBuf, end]);
  const out = path.join(dir, `${manifest.id}-${manifest.version}.edplugin.zip`);
  await writeFile(out, zip);
  const sha256 = createHash('sha256').update(zip).digest('hex');
  console.log(`${out} (${files.length} files, ${zip.length} bytes)`);
  console.log(`sha256: ${sha256}`);
}

// ---------------------------------------------------------------------------
// validation (spec §91: automated review checks, local edition)
// ---------------------------------------------------------------------------

async function validatePlugin(dir) {
  const manifest = await loadManifest(dir);
  const problems = [];

  for (const key of MANIFEST_REQUIRED) {
    if (manifest[key] === undefined) problems.push(`missing required field: ${key}`);
  }
  if (manifest.id && !/^[a-z0-9-]+\.[a-z0-9-]+$/.test(manifest.id)) {
    problems.push(`id must be publisher.name (got "${manifest.id}")`);
  }
  if (manifest.trust && !['trusted', 'community'].includes(manifest.trust)) {
    problems.push(`trust must be "trusted" or "community" (got "${manifest.trust}")`);
  }
  for (const perm of manifest.permissions ?? []) {
    if (!KNOWN_PERMISSIONS.has(perm)) problems.push(`unknown permission: ${perm}`);
  }
  const contributes = manifest.contributes ?? {};
  const idPrefix = `${manifest.id}.`;
  for (const cmd of contributes.commands ?? []) {
    if (!cmd.id?.startsWith(idPrefix)) problems.push(`command id must start with "${idPrefix}": ${cmd.id}`);
  }
  for (const view of contributes.views ?? []) {
    if (!view.id?.startsWith(idPrefix)) problems.push(`view id must start with "${idPrefix}": ${view.id}`);
  }
  for (const key of ['commands', 'views', 'settings', 'searchProviders']) {
    if (contributes[key] !== undefined && !Array.isArray(contributes[key])) {
      problems.push(`contributes.${key} must be an array`);
    }
  }
  if (!existsSync(path.join(dir, 'entry.html'))) problems.push('missing entry.html');
  if (!existsSync(path.join(dir, 'src/main.ts'))) problems.push('missing src/main.ts');

  if (problems.length > 0) {
    console.error(`✗ ${manifest.id} — ${problems.length} problem(s):`);
    for (const p of problems) console.error(`  - ${p}`);
    process.exit(1);
  }
  console.log(`✓ ${manifest.id} v${manifest.version} — manifest valid`);
}

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

async function main() {
  const argv = process.argv.slice(2);
  const [group, command, ...rest] = argv;
  const opts = { dir: undefined, publisher: undefined };
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === '--dir') opts.dir = rest[++i];
    else if (rest[i] === '--publisher') opts.publisher = rest[++i];
    else if (!opts._positional) opts._positional = rest[i];
  }

  if (group === 'plugin' && command === 'create') {
    await create(opts._positional ?? rest[0], opts);
  } else if (group === 'plugin' && command === 'build') {
    const dir = path.resolve(opts._positional ?? rest[0] ?? '.');
    const m = await buildPlugin(dir);
    console.log(`built ${m.id} → ${path.join(dir, 'dist/main.js')}`);
  } else if (group === 'plugin' && command === 'dev') {
    await devPlugin(path.resolve(opts._positional ?? rest[0] ?? '.'));
  } else if (group === 'plugin' && command === 'pack') {
    await packPlugin(path.resolve(opts._positional ?? rest[0] ?? '.'));
  } else if (group === 'plugin' && command === 'validate') {
    await validatePlugin(path.resolve(opts._positional ?? rest[0] ?? '.'));
  } else {
    console.log(`wb — EigenDesk plugin developer CLI

Usage:
  wb plugin create <name> [--dir path] [--publisher id]
  wb plugin build [dir]
  wb plugin dev [dir]
  wb plugin pack [dir]
  wb plugin validate [dir]

Dev plugin dir: ${path.join(dataDir(), 'dev-plugins')}`);
    process.exit(group ? 1 : 0);
  }
}

main().catch((err) => fail(err.stack || String(err)));
