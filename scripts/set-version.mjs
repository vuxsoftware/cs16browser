import { readFileSync, writeFileSync } from 'node:fs';

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error('Usage: bun run version:set <major.minor.patch[-prerelease]>');
}

function update(path, replacement) {
  const source = readFileSync(path, 'utf8');
  const result = replacement(source);
  if (result === source && !source.includes(version)) {
    throw new Error(`Could not update version in ${path}`);
  }
  return { path, source, result };
}

function replaceOne(source, pattern, path) {
  if (!pattern.test(source)) throw new Error(`Version field not found in ${path}`);
  return source.replace(pattern, (_match, before, after) => `${before}${version}${after}`);
}

const files = [
  update('package.json', (source) =>
    replaceOne(source, /("version": ")[^"]+("\s*,)/, 'package.json'),
  ),
  update('src-tauri/tauri.conf.json', (source) => {
    const withVersion = replaceOne(
      source,
      /("version": ")[^"]+("\s*,)/,
      'src-tauri/tauri.conf.json',
    );
    return replaceOne(
      withVersion,
      /("title": "cs16browser v)[^"]+("\s*,)/,
      'src-tauri/tauri.conf.json',
    );
  }),
  update('src-tauri/Cargo.toml', (source) =>
    replaceOne(source, /^(version = ")[^"]+("\s*)$/m, 'src-tauri/Cargo.toml'),
  ),
  update('src-tauri/Cargo.lock', (source) =>
    replaceOne(
      source,
      /(\[\[package\]\]\r?\nname = "cs16browser"\r?\nversion = ")[^"]+("\s*)/,
      'src-tauri/Cargo.lock',
    ),
  ),
];

for (const { path, source, result } of files) {
  if (source !== result) writeFileSync(path, result);
}
console.log(`Set cs16browser version to ${version}.`);
