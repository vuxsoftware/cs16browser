/* Progressive enhancement only: every link already works without this file.
 * It fills in the GitHub star count and, once a release exists, points the
 * download buttons straight at the Windows installer and shows its version.
 * It also opens a FAQ answer when a link points at it. */
const repository = document.body.dataset.repository;

function github(path) {
  return fetch(`https://api.github.com/repos/${repository}${path}`, {
    headers: { Accept: "application/vnd.github+json" },
  }).then((response) => {
    if (!response.ok) throw new Error(`GitHub API ${response.status}`);
    return response.json();
  });
}

if (repository) {
  github("").then((repo) => {
    const count = document.getElementById("star-count");
    if (count && Number.isInteger(repo.stargazers_count)) {
      count.textContent = repo.stargazers_count.toLocaleString();
    }
  }).catch(() => {
    // The badge still links to the repository without a count.
  });

  github("/releases/latest").then((release) => {
    const installer = (release.assets ?? []).find((asset) => /-setup\.exe$/i.test(asset.name));
    if (installer) {
      for (const link of document.querySelectorAll("[data-download]")) {
        link.href = installer.browser_download_url;
      }
    }
    for (const el of document.querySelectorAll("[data-version]")) el.textContent = release.tag_name;
    for (const el of document.querySelectorAll("[data-version-line]")) el.hidden = false;
  }).catch(() => {
    // No release yet, or the API is unavailable: the buttons open the releases page.
  });
}

/* A link to a FAQ entry (like #smartscreen) should land on it open. */
function openLinkedAnswer() {
  const target = location.hash && document.getElementById(location.hash.slice(1));
  if (target instanceof HTMLDetailsElement) target.open = true;
}
window.addEventListener("hashchange", openLinkedAnswer);
openLinkedAnswer();
