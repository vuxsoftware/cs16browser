import { expect, it, vi } from "vitest";
import { mount, tick, unmount } from "svelte";
import { version } from "../../package.json";

it("shows the app version and closes on the Close button", async () => {
  const About = (await import("./About.svelte")).default;
  const onclose = vi.fn();
  const app = mount(About, { target: document.body, props: { onclose } });
  await tick();

  expect(document.body.textContent).toContain(`v${version}`);

  const close = [...document.body.querySelectorAll("button")].find(
    (b) => b.textContent === "Close",
  );
  close?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  expect(onclose).toHaveBeenCalled();

  unmount(app);
});
