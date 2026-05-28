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
    <main class="app-shell">
      <h1 class="app-title">DivoraVoice</h1>
      <p class="app-subtitle">
        Phase 0 scaffold. Design system + app shell land in Phase 1.
      </p>
      <button type="button" onClick={handlePing} class="app-button">
        Ping backend
      </button>
      {pong() && (
        <p class="app-pong">
          backend says: {pong()}
        </p>
      )}
    </main>
  );
}
