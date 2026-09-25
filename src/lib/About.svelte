<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import { version } from "../../package.json";
  import { cs16 } from "$lib/cs16";

  let { onclose }: { onclose: () => void } = $props();

  /* A plain `<a target="_blank">` cannot open the system browser here: the
   * webview has no arbitrary-URL permission (see AGENTS.md), so it silently
   * did nothing. `openRepositoryPage` opens this one fixed URL from Rust. */
  function openRepository(): void {
    void cs16.openRepositoryPage();
  }
</script>

<Dialog title="About cs16browser" width={880} height={490} buttons={[{ label: "Close" }]} {onclose}>
  <div class="about-content">
    <h2>cs16browser <span>v{version}</span></h2>
    <p>
      A community made browser for Counter-Strike 1.6 servers. It checks server responses and
      highlights suspicious listings so you can choose where to play.
    </p>
    <p>
      Created by <strong>vuxsoftware</strong>. Open source, free to use, and intended to stay free.
    </p>
    <p class="repository">
      GitHub repository: <button type="button" class="link" onclick={openRepository}
        >github.com/vuxsoftware/cs16browser</button
      >
    </p>
    <p class="legal">
      Unofficial project. Not affiliated with, sponsored by, or endorsed by Valve Corporation.
      Counter-Strike and Steam are trademarks of Valve Corporation.
    </p>
  </div>
</Dialog>

<style>
  .about-content {
    position: absolute;
    top: 86px;
    left: 52px;
    right: 52px;
    font-size: 20px;
    line-height: 1.35;
    color: var(--text);
  }
  h2 {
    margin: 0 0 17px;
    font-size: 27px;
    font-weight: normal;
    color: var(--text-bright);
  }
  h2 span {
    color: var(--text-dim);
    font-size: 20px;
  }
  p {
    margin: 0 0 16px;
  }
  strong {
    font-weight: normal;
    color: var(--text-bright);
  }
  .repository,
  .legal {
    color: var(--text-dim);
  }
  .repository .link {
    color: inherit;
    font: inherit;
    background: none;
    border: none;
    padding: 0;
    text-decoration: underline;
    cursor: default;
  }
  .legal {
    font-size: 17px;
    margin-top: 23px;
  }
</style>
