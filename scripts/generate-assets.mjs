#!/usr/bin/env node
/**
 * Render pipeline for brand assets (docs/assets/*.svg → PNGs + app icons).
 *
 * All artwork is source-controlled as SVG; PNG derivatives and Tauri icon
 * sizes are generated — never hand-edited. Requires `rsvg-convert`
 * (librsvg) and ImageMagick `magick|convert` (for the multires .ico only).
 *
 * Usage: node scripts/generate-assets.mjs
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const assets = join(root, 'docs', 'assets');
const icons = join(root, 'apps', 'desktop', 'src-tauri', 'icons');

function rsvg(svg, out, width, height) {
  const args = [svg, '-o', out];
  if (width) args.push('-w', String(width));
  if (height) args.push('-h', String(height));
  execFileSync('rsvg-convert', args, { stdio: 'inherit' });
  console.log(`rendered ${out}`);
}

/** Docs-facing renders. Sizes match what README/GitHub expects. */
const docsRenders = [
  ['logo.svg', 'logo.png', 1024, 1024],
  ['banner.svg', 'banner.png', 2560, 1280],
  ['social-preview.svg', 'social-preview.png', 1280, 640],
  ['hero.svg', 'hero.png', 1600, 1000],
];

/** Tauri icon renders (Linux bundles use these; .icns/.ico kept prebuilt). */
const iconRenders = [
  [32, '32x32.png'],
  [64, '64x64.png'],
  [128, '128x128.png'],
  [256, '128x128@2x.png'],
  [512, 'icon.png'],
  [30, 'Square30x30Logo.png'],
  [44, 'Square44x44Logo.png'],
  [71, 'Square71x71Logo.png'],
  [89, 'Square89x89Logo.png'],
  [107, 'Square107x107Logo.png'],
  [142, 'Square142x142Logo.png'],
  [150, 'Square150x150Logo.png'],
  [284, 'Square284x284Logo.png'],
  [310, 'Square310x310Logo.png'],
  [50, 'StoreLogo.png'],
];

for (const [svg, png, w, h] of docsRenders) {
  rsvg(join(assets, svg), join(assets, png), w, h);
}

mkdirSync(icons, { recursive: true });
for (const [size, name] of iconRenders) {
  rsvg(join(assets, 'logo.svg'), join(icons, name), size, size);
}

// Multi-resolution .ico — ImageMagick if available; otherwise keep the
// checked-in file (Windows builds are not part of the 1.0 release matrix).
const magick = ['magick', 'convert'].find((bin) => {
  try {
    execFileSync(bin, ['-version'], { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
});
if (magick) {
  const ico = join(icons, 'icon.ico');
  try {
    execFileSync(
      magick,
      [join(icons, 'icon.png'), '-define', 'icon:auto-resize=16,32,48,256', ico],
      { stdio: 'inherit' },
    );
    console.log(`rendered ${ico}`);
  } catch {
    console.warn('skipping icon.ico (ImageMagick failed; checked-in file kept)');
  }
} else {
  console.warn('skipping icon.ico (ImageMagick not found; checked-in file kept)');
}

// Sanity: every PNG we claim to generate must exist and be non-empty.
for (const f of [...docsRenders.map(([, p]) => join(assets, p)), ...iconRenders.map(([, p]) => join(icons, p))]) {
  if (!existsSync(f)) throw new Error(`missing render: ${f}`);
}
console.log('all assets rendered');
