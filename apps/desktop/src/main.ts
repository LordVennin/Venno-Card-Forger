import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { runParityIfRequested } from "./lib/parity";

void runParityIfRequested();

export default mount(App, { target: document.getElementById("app")! });
