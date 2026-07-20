// SpeakScreen — inline rename of a cloned voice (v1.43.0).
//
// The store unit tests cover the rename *action*; these cover the parts only a
// rendered DOM can prove: that the pencil swaps the card into a **focused**
// editor, that Enter commits and Escape abandons, and that the card goes back
// to showing a (renamed) voice afterwards.

import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, waitFor } from "@solidjs/testing-library";

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

import { AppProvider } from "../stores/app";
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
