const KEY = "cs16browser.autoUpdate";

export function autoUpdateEnabled(): boolean {
  try {
    return localStorage.getItem(KEY) !== "false";
  } catch {
    return true;
  }
}

export function setAutoUpdateEnabled(enabled: boolean): void {
  try {
    localStorage.setItem(KEY, String(enabled));
  } catch {
    /* An unavailable storage area must not break Settings. */
  }
}
