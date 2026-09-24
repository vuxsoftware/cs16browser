import { readFileSync } from 'node:fs';

const config = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const endpoint = config.plugins?.updater?.endpoints?.[0];
if (!endpoint || endpoint.includes('/OWNER/') || !endpoint.startsWith('https://github.com/')) {
  throw new Error('Set the real GitHub release URL in tauri.conf.json before releasing.');
}
if (process.env.GITHUB_REPOSITORY && endpoint !== `https://github.com/${process.env.GITHUB_REPOSITORY}/releases/latest/download/latest.json`) {
  throw new Error(`Updater URL must point at the release feed for ${process.env.GITHUB_REPOSITORY}.`);
}
if (!process.env.TAURI_SIGNING_PRIVATE_KEY) {
  throw new Error('TAURI_SIGNING_PRIVATE_KEY GitHub secret is required for signed updater artifacts.');
}
console.log(`Updater endpoint: ${endpoint}`);
