<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";

  let version = $state("");
  // Outside Tauri (plain `vite` in a browser) there is no IPC; leave it blank.
  getVersion().then((v) => (version = v)).catch(() => {});
</script>

<main>
  <h1>Cardsmith</h1>
  <p>Phase 0 shell{version ? ` · v${version}` : ""}</p>
</main>

<style>
  main {
    display: grid;
    place-content: center;
    height: 100vh;
    text-align: center;
  }
  h1 { margin: 0; font-weight: 600; }
  p { color: #9a9ca3; }
</style>
