import { readFileSync } from 'node:fs';

const packageVersion = JSON.parse(readFileSync('package.json', 'utf8')).version;
const tauriVersion = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).version;
const cargoVersion = /^version\s*=\s*"([^"]+)"/m.exec(readFileSync('src-tauri/Cargo.toml', 'utf8'))?.[1];
if (!cargoVersion || packageVersion !== tauriVersion || packageVersion !== cargoVersion) {
  throw new Error(`Version mismatch: package=${packageVersion}, Tauri=${tauriVersion}, Cargo=${cargoVersion}`);
}
const tag = process.env.GITHUB_REF_NAME;
if (tag?.startsWith('v') && tag !== `v${packageVersion}`) {
  throw new Error(`Release tag ${tag} must match v${packageVersion}`);
}
console.log(`Version ${packageVersion} is consistent.`);
