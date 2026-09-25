import { readFileSync } from 'node:fs';

const packageVersion = JSON.parse(readFileSync('package.json', 'utf8')).version;
const tauriConfig = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const tauriVersion = tauriConfig.version;
const cargoVersion = /^version\s*=\s*"([^"]+)"/m.exec(readFileSync('src-tauri/Cargo.toml', 'utf8'))?.[1];
const lockVersion = /\[\[package\]\]\r?\nname = "cs16browser"\r?\nversion = "([^"]+)"/.exec(readFileSync('src-tauri/Cargo.lock', 'utf8'))?.[1];
const windowTitle = tauriConfig.app?.windows?.[0]?.title;
if (packageVersion !== tauriVersion || packageVersion !== cargoVersion || packageVersion !== lockVersion || windowTitle !== `cs16browser v${packageVersion}`) {
  throw new Error(`Version mismatch: package=${packageVersion}, Tauri=${tauriVersion}, Cargo=${cargoVersion}, lock=${lockVersion}, title=${windowTitle}`);
}
const tag = process.env.GITHUB_REF_NAME;
if (tag?.startsWith('v') && tag !== `v${packageVersion}`) {
  throw new Error(`Release tag ${tag} must match v${packageVersion}`);
}
console.log(`Version ${packageVersion} is consistent.`);
