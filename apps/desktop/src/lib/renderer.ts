import wasmUrl from "../wasm/cardsmith.wasm?url";
import { Core } from "./core";

let ready: Promise<Core> | undefined;

/** The app's single core instance. */
export function getCore(): Promise<Core> {
  ready ??= fetch(wasmUrl)
    .then((response) => response.arrayBuffer())
    .then((bytes) => Core.load(bytes));
  return ready;
}
