<script lang="ts">
  /* TitleBar.svelte — the caption strip above the tabs: app glyph, a caption,
   * and the boxed close button. Shared by the main window and every dialog.
   *
   * Measured (main window and Game Info alike): glyph ink x13..44 / y17..47,
   * caption ink from x56 with its cap at y24..43, close box 36x36 with its
   * right edge 11px in from the frame and its top at y15/16. The main window
   * adds minimize, maximize and settings boxes — same 36x36 bevel,
   * 6px apart.
   */
  import Icon from "./Icon.svelte";

  interface Props {
    caption: string;
    onclose: () => void;
    /** The main window's strip drags the native window; a dialog's strip
     * drags the dialog itself, so it hands its own pointer handler in. */
    ondragstart?: (event: PointerEvent) => void;
    /** Main window only: minimize the native window. */
    onminimize?: () => void;
    /** Main window only: toggle native maximization. */
    onmaximize?: () => void;
    /** Main window only: open the Settings dialog. */
    onsettings?: () => void;
    onabout?: () => void;
  }

  let { caption, onclose, ondragstart, onminimize, onmaximize, onsettings, onabout }: Props =
    $props();
</script>

<div
  class="titlebar"
  data-tauri-drag-region={ondragstart ? undefined : true}
  onpointerdown={ondragstart}
  role="presentation"
>
  <span class="glyph"><img src="/vuxsoftwareicon.png" alt="" width="32" height="32" /></span>
  <span class="caption">{caption}</span>
  {#if onabout}
    <button
      class="tbtn about"
      aria-label="About"
      title="About cs16browser"
      onpointerdown={(e) => e.stopPropagation()}
      onclick={(e) => {
        e.stopPropagation();
        onabout();
      }}><Icon name="info" /></button
    >
  {/if}
  {#if onsettings}
    <button
      class="tbtn settings"
      aria-label="Settings"
      title="Settings"
      onpointerdown={(e) => e.stopPropagation()}
      onclick={(e) => {
        e.stopPropagation();
        onsettings();
      }}
    >
      <Icon name="gear" />
    </button>
  {/if}
  {#if onminimize}
    <button
      class="tbtn minimize"
      aria-label="Minimize"
      title="Minimize"
      onpointerdown={(e) => e.stopPropagation()}
      onclick={(e) => {
        e.stopPropagation();
        onminimize();
      }}
    >
      <Icon name="minimize" />
    </button>
  {/if}
  {#if onmaximize}
    <button
      class="tbtn maximize"
      aria-label="Maximize or restore"
      title="Maximize or restore"
      onpointerdown={(e) => e.stopPropagation()}
      onclick={(e) => {
        e.stopPropagation();
        onmaximize();
      }}
    >
      <Icon name="maximize" />
    </button>
  {/if}
  <button
    class="tbtn close"
    aria-label="Close"
    onpointerdown={(e) => e.stopPropagation()}
    onclick={(e) => {
      e.stopPropagation();
      onclose();
    }}
  >
    <Icon name="close" />
  </button>
</div>

<style>
  .titlebar {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 90px;
  }
  /* Children must not swallow the drag: Tauri only starts a drag when the
   * pressed element itself carries the attribute. */
  .glyph,
  .caption {
    pointer-events: none;
  }
  .glyph {
    position: absolute;
    left: 13px;
    top: 16px;
    color: var(--text-bright);
  }
  .glyph img {
    display: block;
    object-fit: contain;
  }
  .caption {
    position: absolute;
    left: 55px;
    top: 16px;
    height: 34px;
    display: flex;
    align-items: center;
    font-size: var(--fs-chrome);
    color: var(--text-bright);
    white-space: nowrap;
  }
  .tbtn {
    position: absolute;
    right: 11px;
    top: 15px;
    width: 36px;
    height: 36px;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--panel-bg);
    border-top: 1px solid var(--bevel-light);
    border-left: 1px solid var(--bevel-light);
    border-bottom: 1px solid var(--inset);
    border-right: 1px solid var(--inset);
    color: #c1bfbd;
    outline: none;
    cursor: default;
  }
  .tbtn.maximize {
    right: 53px;
  }
  .tbtn.minimize {
    right: 95px;
  }
  .tbtn.settings {
    right: 137px;
  }
  .tbtn.about {
    right: 179px;
  }
  .tbtn:hover {
    color: var(--text-bright);
  }
  .tbtn:active {
    border-top-color: var(--inset);
    border-left-color: var(--inset);
    border-bottom-color: var(--bevel-light);
    border-right-color: var(--bevel-light);
  }
</style>
