// Loads the app without an inline script, so the page's CSP can forbid them.
import init from "./sa-forge-signer-grant.js";

await init({ module_or_path: "./sa-forge-signer-grant_bg.wasm" });
