// The page's only JavaScript besides wasm-bindgen's glue: Wallet Standard discovery, signing,
// JSON-RPC and time. No dependencies; Rust builds and checks every transaction.

const wallets = [];
let wallet = null;
let account = null;

// Wallet Standard: wallets register through this API, whether they load before or after the page.
const api = Object.freeze({
  register(...ws) {
    for (const w of ws) if (!wallets.includes(w)) wallets.push(w);
    return () => {};
  },
});
window.addEventListener("wallet-standard:register-wallet", (event) => {
  try {
    event.detail(api);
  } catch (err) {
    console.error("wallet registration failed", err);
  }
});
window.dispatchEvent(new CustomEvent("wallet-standard:app-ready", { detail: api }));

function signers() {
  return wallets.filter(
    (w) => w.features["standard:connect"] && w.features["solana:signTransaction"],
  );
}

export function walletNames() {
  return JSON.stringify(signers().map((w) => w.name));
}

export async function connect(name) {
  const w = signers().find((x) => x.name === name);
  if (!w) throw new Error(`wallet not found: ${name}`);
  const { accounts } = await w.features["standard:connect"].connect();
  if (!accounts || accounts.length === 0) throw new Error("the wallet returned no account");
  wallet = w;
  account = accounts[0];
  return account.address;
}

export async function disconnect() {
  const w = wallet;
  wallet = null;
  account = null;
  if (w && w.features["standard:disconnect"]) await w.features["standard:disconnect"].disconnect();
}

// Takes and returns a serialized transaction; the wallet adds its signature.
export async function signTransaction(tx) {
  if (!wallet || !account) throw new Error("no wallet connected");
  const out = await wallet.features["solana:signTransaction"].signTransaction({
    account,
    transaction: tx,
  });
  const signed = out && out[0] && out[0].signedTransaction;
  if (!signed) throw new Error("the wallet returned no transaction");
  return signed;
}

export async function rpc(url, method, paramsJson) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: JSON.parse(paramsJson) }),
  });
  if (!res.ok) throw new Error(`RPC ${method}: HTTP ${res.status}`);
  const body = await res.json();
  if (body.error) throw new Error(`RPC ${method}: ${body.error.message ?? JSON.stringify(body.error)}`);
  return JSON.stringify(body.result);
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function nowSecs() {
  return BigInt(Math.floor(Date.now() / 1000));
}

export function isoTime(secs) {
  return new Date(Number(secs) * 1000).toISOString().replace(".000Z", "Z");
}
