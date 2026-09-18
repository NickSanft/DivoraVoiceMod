// SpeakScreen — inline rename of a cloned voice (v1.43.0).
//
// The store unit tests cover the rename *action*; these cover the parts only a
// rendered DOM can prove: that the pencil swaps the card into a **focused**
// editor, that Enter commits and Escape abandons, and that the card goes back
// to showing a (renamed) voice afterwards.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, waitFor, within } from "@solidjs/testing-library";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {
    /* unlisten no-op */
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
}));
vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(async () => undefined),
}));

import { AppProvider, useApp } from "../stores/app";
import { SpeakScreen } from "./SpeakScreen";

/** One installed preset (so the "Your voices" section renders) + one clone. */
function seedInvoke(cloned: { id: string; name: string }[]) {
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    switch (cmd) {
      case "list_tts_voices":
        return [
          { id: "af_heart", name: "Aria", lang: "en-us", installed: true },
        ];
      case "list_cloned_voices":
        return cloned.map((v) => ({ ...v, baseName: "Puck", engine: "voxcpm" }));
      case "rename_cloned_voice": {
        const { id, name } = args as { id: string; name: string };
        const hit = cloned.find((v) => v.id === id);
        if (hit) hit.name = name;
        return undefined;
      }
      case "clone_models_status":
        return { ready: true };
      default:
        return undefined;
    }
  });
}

function setupScreen() {
  return render(() => (
    <AppProvider>
      <SpeakScreen />
    </AppProvider>
  ));
}

describe("SpeakScreen — rename a cloned voice (v1.43.0)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("the pencil opens a focused, pre-filled editor", async () => {
    seedInvoke([{ id: "my-voice", name: "My Voice" }]);
    const { findByRole, getByRole } = setupScreen();

    const pencil = await findByRole("button", { name: "Rename My Voice" });
    fireEvent.click(pencil);

    // Scoped by role: the pencil button carries the same aria-label as the
    // editor, so a label-only query would match either one.
    const input = getByRole("textbox", {
      name: "Rename My Voice",
    }) as HTMLInputElement;
    expect(input.value).toBe("My Voice");
    // The editor autofocuses (deferred a microtask past insertion), so the
    // user can type straight away without clicking into the field.
    await waitFor(() => expect(document.activeElement).toBe(input));
  });

  it("Enter commits the rename and closes the editor", async () => {
    seedInvoke([{ id: "my-voice", name: "My Voice" }]);
    const { findByRole, getByRole } = setupScreen();

    fireEvent.click(await findByRole("button", { name: "Rename My Voice" }));
    const input = getByRole("textbox", {
      name: "Rename My Voice",
    }) as HTMLInputElement;
    fireEvent.input(input, { target: { value: "  Static Wraith  " } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("rename_cloned_voice", {
        id: "my-voice",
        name: "Static Wraith", // trimmed on the way out
      }),
    );
    // The editor closes and the card shows the new name.
    await findByRole("button", { name: "Rename Static Wraith" });
  });

  it("Escape abandons the edit without calling the backend", async () => {
    seedInvoke([{ id: "my-voice", name: "My Voice" }]);
    const { findByRole, getByRole, queryByRole } = setupScreen();

    fireEvent.click(await findByRole("button", { name: "Rename My Voice" }));
    const input = getByRole("textbox", {
      name: "Rename My Voice",
    }) as HTMLInputElement;
    fireEvent.input(input, { target: { value: "Discarded" } });
    fireEvent.keyDown(input, { key: "Escape" });

    // The editor (textbox) is gone — note the rename BUTTON shares its
    // aria-label, so this must be scoped by role to mean anything.
    await waitFor(() =>
      expect(queryByRole("textbox", { name: "Rename My Voice" })).toBeNull(),
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      "rename_cloned_voice",
      expect.anything(),
    );
    // Still the original name.
    await findByRole("button", { name: "Rename My Voice" });
  });

  it("closing the editor hands focus back to the pencil", async () => {
    seedInvoke([{ id: "my-voice", name: "My Voice" }]);
    const { findByRole, getByRole } = setupScreen();

    const pencil = await findByRole("button", { name: "Rename My Voice" });
    fireEvent.click(pencil);
    const input = getByRole("textbox", {
      name: "Rename My Voice",
    }) as HTMLInputElement;
    fireEvent.keyDown(input, { key: "Escape" });

    // Without this a keyboard user is dumped back to <body> and has to tab in
    // from the top of the document again.
    await waitFor(() =>
      expect(document.activeElement).toBe(
        getByRole("button", { name: "Rename My Voice" }),
      ),
    );
  });

  it("an empty name is not committed", async () => {
    seedInvoke([{ id: "my-voice", name: "My Voice" }]);
    const { findByRole, getByRole } = setupScreen();

    fireEvent.click(await findByRole("button", { name: "Rename My Voice" }));
    const input = getByRole("textbox", {
      name: "Rename My Voice",
    }) as HTMLInputElement;
    fireEvent.input(input, { target: { value: "   " } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(invokeMock).not.toHaveBeenCalledWith(
      "rename_cloned_voice",
      expect.anything(),
    );
    // The editor stays open so the name can be corrected.
    expect(getByRole("textbox", { name: "Rename My Voice" })).toBeTruthy();
  });
});

describe("SpeakScreen — tap-to-audition previews (v1.47.0)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("the preset card keeps its radio semantics after the restructure", async () => {
    seedInvoke([]);
    const { findByRole } = setupScreen();
    // The card was split from ONE role=radio button into a row (radio +
    // sibling audition button), because role=radio is a leaf role and a nested
    // button is unreachable to assistive tech. The radio must survive that.
    const radio = await findByRole("radio", { name: /Aria/ });
    expect(radio.getAttribute("aria-checked")).toBeTruthy();
  });

  it("exposes a Preview button per preset voice", async () => {
    seedInvoke([]);
    const { findByRole } = setupScreen();
    const play = await findByRole("button", { name: "Preview Aria" });
    expect(play).toBeTruthy();
  });

  it("disables the preview with a reason when the engine is stopped", async () => {
    seedInvoke([]);
    const { findByRole } = setupScreen();
    const play = (await findByRole("button", {
      name: "Preview Aria",
    })) as HTMLButtonElement;
    // The engine is stopped in this harness. Auditioning with no output stream
    // would render into silence, which reads as a broken feature — so the
    // button says why instead of doing nothing.
    expect(play.disabled).toBe(true);
    expect(play.getAttribute("title")).toMatch(/start the engine/i);
  });

  it("the audition button is a SIBLING of the radio, never a descendant", async () => {
    seedInvoke([]);
    const { findByRole } = setupScreen();
    const radio = await findByRole("radio", { name: /Aria/ });
    const play = await findByRole("button", { name: "Preview Aria" });
    // role=radio is a leaf role: a nested button would be unreachable to
    // assistive tech, which is the whole reason the preset card was split from
    // a single button into a row.
    expect(radio.contains(play)).toBe(false);
  });
});

describe("SpeakScreen — Critter Chatter voices (v1.50.0)", () => {
  const CRITTERS = [
    { id: "babble:bright", name: "Bright", lang: "en-us", installed: true, engine: "babble" },
    { id: "babble:mellow", name: "Mellow", lang: "en-us", installed: true, engine: "babble" },
    { id: "babble:gruff", name: "Gruff", lang: "en-us", installed: true, engine: "babble" },
  ];
  const aria = (installed: boolean) => ({
    id: "af_heart",
    name: "Aria",
    lang: "en-us",
    installed,
    engine: "kokoro",
  });

  function seedVoices(voices: object[]) {
    invokeMock.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "list_tts_voices":
          return voices;
        case "list_cloned_voices":
        case "list_speak_clips":
          return [];
        case "clone_models_status":
          return { ready: true };
        case "preview_voice":
          return 1.0;
        default:
          return undefined;
      }
    });
  }

  /** Like setupScreen, but hands back the store so a test can start the engine. */
  function setupWithApp() {
    let captured: ReturnType<typeof useApp> | null = null;
    function Inner() {
      captured = useApp();
      return <SpeakScreen />;
    }
    const utils = render(() => (
      <AppProvider>
        <Inner />
      </AppProvider>
    ));
    return { ...utils, app: () => captured! };
  }

  beforeEach(() => {
    invokeMock.mockReset();
    try {
      window.localStorage.clear();
    } catch {
      /* fine */
    }
  });

  it("renders its own labelled radiogroup, separate from the presets, with no Soon badge", async () => {
    seedVoices([aria(false), ...CRITTERS]);
    const { findByRole, getByRole } = setupScreen();

    const group = await findByRole("radiogroup", { name: "Critter Chatter" });
    expect(within(group).getAllByRole("radio").map((r) => r.textContent)).toEqual([
      "Bright",
      "Mellow",
      "Gruff",
    ]);
    const caption = document.getElementById(group.getAttribute("aria-describedby")!);
    expect(caption?.textContent).toMatch(/built in, no download/i);
    // Always installed, so never promised as "Soon" — unlike the missing preset.
    expect(within(group).queryByText("Soon")).toBeNull();

    const presets = getByRole("radiogroup", { name: "Preset voice" });
    expect(within(presets).queryByRole("radio", { name: /Bright/ })).toBeNull();
    expect(within(presets).getByText("Soon")).toBeTruthy();
  });

  it("selecting one checks it and persists the id", async () => {
    seedVoices([aria(false), ...CRITTERS]);
    const { findByRole, getByRole } = setupScreen();

    const gruff = await findByRole("radio", { name: /Gruff/ });
    // Wait for the default selection, so the click is not overwritten by it.
    await waitFor(() =>
      expect(getByRole("radio", { name: /Aria/ }).getAttribute("aria-checked")).toBe("true"),
    );
    fireEvent.click(gruff);

    expect(gruff.getAttribute("aria-checked")).toBe("true");
    expect(getByRole("radio", { name: /Aria/ }).getAttribute("aria-checked")).toBe("false");
    expect(window.localStorage.getItem("divora.ttsVoice")).toContain("babble:gruff");
  });

  it("auditions with the engine running even when Kokoro is missing", async () => {
    seedVoices([aria(false), ...CRITTERS]);
    const { findByRole, getByRole, app } = setupWithApp();
    app().setEngineRunning(true);

    const play = (await findByRole("button", { name: "Preview Bright" })) as HTMLButtonElement;
    expect(play.disabled).toBe(false);
    // The missing preset stays disabled, and says why.
    const ariaPlay = getByRole("button", { name: "Preview Aria" }) as HTMLButtonElement;
    expect(ariaPlay.disabled).toBe(true);
    expect(ariaPlay.getAttribute("title")).toMatch(/isn't installed/i);

    fireEvent.click(play);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "preview_voice",
        expect.objectContaining({ voiceId: "babble:bright" }),
      ),
    );
  });

  it("without Kokoro: the banner points at Critter Chatter, cloning hides, Saved clips stays", async () => {
    seedVoices([aria(false), ...CRITTERS]);
    const { findByRole, getByText, queryByText, queryByRole } = setupScreen();
    await findByRole("radiogroup", { name: "Critter Chatter" }); // voices loaded

    const banner = getByText(/aren't installed in this build/i).closest("[role=status]");
    expect(banner?.textContent).toMatch(/Critter Chatter/);
    // Cloning renders through a Kokoro base, so it must not be offered here.
    expect(queryByText("Your voices")).toBeNull();
    expect(queryByRole("button", { name: /Pick a clip/ })).toBeNull();
    // But there is something that can speak, so its clips stay reachable.
    expect(getByText("Saved clips")).toBeTruthy();
  });

  it("with Kokoro installed: no banner, and cloning is offered", async () => {
    seedVoices([aria(true), ...CRITTERS]);
    const { findByText, queryByText } = setupScreen();

    await findByText("Your voices");
    expect(queryByText(/aren't installed/i)).toBeNull();
    expect(queryByText("Saved clips")).toBeTruthy();
  });
});
