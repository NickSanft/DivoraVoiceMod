import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

export default function App() {
  const [pong, setPong] = createSignal<string>("");

  const handlePing = async () => {
    try {
      const reply = await invoke<string>("ping");
      setPong(reply);
    } catch (err) {
      setPong(`error: ${String(err)}`);
    }
  };

  return (
    <main class="flex min-h-screen flex-col items-center justify-center bg-zinc-950 text-zinc-100 p-8">
      <h1 class="text-4xl font-semibold tracking-tight mb-3">Divora</h1>
      <p class="text-zinc-400 mb-8">
        Phase 0 scaffold. Audio engine lands in Phase 1.
      </p>
      <button
        type="button"
        onClick={handlePing}
        class="rounded-md bg-indigo-500 px-4 py-2 text-sm font-medium hover:bg-indigo-400 transition"
      >
        Ping backend
      </button>
      {pong() && (
        <p class="mt-4 text-emerald-400 text-sm font-mono">
          backend says: {pong()}
        </p>
      )}
    </main>
  );
}
