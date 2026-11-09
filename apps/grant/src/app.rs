//! The page: check the network, connect the wallet, read the profile, then grant or revoke one key.
//! Every transaction is built here from the form and re-read state, never from pasted input.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::bridge::{self, iso_time, now_secs};
use crate::perms::{self, DRAIN_SOL_VAULT, PRESETS, SAGE};
use crate::profile::{
    self, ADD_KEYS, Grant, Profile, REMOVE_KEYS, SAGE_PROGRAM, add_keys, authority_index,
    decode_profile, key_at, remove_keys,
};
use crate::rpc::{self, DEFAULT_RPC, EXPLORER};
use crate::tx::{
    Instruction, Pubkey, check_signed, compile_message, encode_pubkey, parse_pubkey,
    unsigned_transaction,
};

/// Expiry choices in days; -1 is the program's "never expires".
const EXPIRY_DAYS: &[i64] = &[-1, 7, 30, 60, 90];
const NEVER: i64 = -1;
const DAY_SECS: i64 = 86_400;

#[derive(Clone)]
enum Line {
    Info(String),
    Error(String),
    Link(String, String),
}

#[derive(Clone)]
struct Pending {
    title: String,
    lines: Vec<String>,
    payer: Pubkey,
    profile: String,
    /// The RPC it was reviewed against, after a genesis check.
    rpc_url: String,
    ix: Instruction,
    change: Change,
    logs: Vec<String>,
}

/// What a reviewed transaction changes, to check before signing and report after it lands.
#[derive(Clone, Copy)]
enum Change {
    /// The key a grant adds, to report its index.
    Grant(Pubkey),
    /// The index a revoke removes, and the key it must still hold.
    Revoke(u16, Pubkey),
}

#[derive(Clone, Copy)]
struct State {
    rpc_url: RwSignal<String>,
    network: RwSignal<Option<Result<(), String>>>,
    wallets: RwSignal<Vec<String>>,
    wallet: RwSignal<Option<String>>,
    profile_addr: RwSignal<String>,
    candidates: RwSignal<Vec<String>>,
    profile: RwSignal<Option<Profile>>,
    new_key: RwSignal<String>,
    bits: RwSignal<u64>,
    drain: RwSignal<bool>,
    days: RwSignal<String>,
    pending: RwSignal<Option<Pending>>,
    busy: RwSignal<bool>,
    log: RwSignal<Vec<Line>>,
}

impl State {
    fn new() -> Self {
        Self {
            rpc_url: RwSignal::new(DEFAULT_RPC.into()),
            network: RwSignal::new(None),
            wallets: RwSignal::new(Vec::new()),
            wallet: RwSignal::new(None),
            profile_addr: RwSignal::new(String::new()),
            candidates: RwSignal::new(Vec::new()),
            profile: RwSignal::new(None),
            new_key: RwSignal::new(String::new()),
            bits: RwSignal::new(0),
            drain: RwSignal::new(false),
            days: RwSignal::new("7".into()),
            pending: RwSignal::new(None),
            busy: RwSignal::new(false),
            log: RwSignal::new(Vec::new()),
        }
    }

    fn say(self, line: Line) {
        self.log.update(|l| l.push(line));
    }

    /// Runs one action at a time, logging its error.
    fn run(self, work: impl Future<Output = Result<(), String>> + 'static) {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            if let Err(e) = work.await {
                self.say(Line::Error(e));
            }
            self.busy.set(false);
        });
    }

    fn wallet_key(self) -> Result<Pubkey, String> {
        parse_pubkey(
            &self
                .wallet
                .get_untracked()
                .ok_or("connect a wallet first")?,
        )
    }
}

async fn check_network(s: State) -> Result<(), String> {
    checked_url(s).await.map(|_| ())
}

/// The RPC URL in the field, once its genesis hash is Z.ink testnet's. Every read and transaction
/// step takes its URL from here, so a URL edited after connecting is never used unchecked.
async fn checked_url(s: State) -> Result<String, String> {
    let url = s.rpc_url.get_untracked();
    let result = rpc::check_network(&url).await;
    s.network.set(Some(result.clone()));
    result.map(|()| url)
}

async fn load_profile(s: State) -> Result<Profile, String> {
    let address = s.profile_addr.get_untracked();
    parse_pubkey(&address)?;
    let data = rpc::account_data(&checked_url(s).await?, address.trim()).await?;
    let profile = decode_profile(&data)?;
    s.profile.set(Some(profile.clone()));
    Ok(profile)
}

async fn connect(s: State, name: String) -> Result<(), String> {
    let url = checked_url(s).await?;
    let address = bridge::connect(&name).await?;
    s.wallet.set(Some(address.clone()));
    s.say(Line::Info(format!("connected {name}: {address}")));
    if s.profile_addr.get_untracked().trim().is_empty() {
        let found = rpc::find_profiles(&url, &address).await?;
        if let [only] = found.as_slice() {
            s.profile_addr.set(only.clone());
            load_profile(s).await?;
        } else if found.is_empty() {
            s.say(Line::Info(
                "no profile found with this wallet as one of its first keys; paste its address"
                    .into(),
            ));
        }
        s.candidates.set(found);
    }
    Ok(())
}

fn grant_permissions(s: State) -> Result<u64, String> {
    let mut permissions = s.bits.get_untracked();
    if s.drain.get_untracked() {
        permissions |= 1 << DRAIN_SOL_VAULT;
    }
    if permissions == 0 {
        return Err("choose at least one permission".into());
    }
    Ok(permissions)
}

/// The key's `expire_time` for an expiry choice: -1, or `now` plus that many days.
fn expire_time(choice: &str, now: i64) -> Result<i64, String> {
    let days: i64 = choice.trim().parse().map_err(|_| "choose an expiry")?;
    if !EXPIRY_DAYS.contains(&days) {
        return Err("choose an expiry from the list".into());
    }
    if days == NEVER {
        return Ok(NEVER);
    }
    days.checked_mul(DAY_SECS)
        .and_then(|d| now.checked_add(d))
        .ok_or_else(|| "expiry overflow".into())
}

fn expiry_text(expire_time: i64) -> String {
    if expire_time == NEVER {
        "never expires".into()
    } else {
        format!("expires {}", iso_time(expire_time))
    }
}

async fn prepare_grant(s: State) -> Result<(), String> {
    s.pending.set(None);
    let wallet = s.wallet_key()?;
    let profile = load_profile(s).await?;
    let now = now_secs();
    let auth = authority_index(&profile, &wallet, ADD_KEYS, now)?;
    let new_key = parse_pubkey(&s.new_key.get_untracked())?;
    if new_key == wallet {
        return Err("the session key must not be the wallet itself".into());
    }
    if let Some(i) = profile
        .keys
        .iter()
        .position(|k| k.key == new_key && !k.expired(now))
    {
        return Err(format!(
            "that key is already on the profile at index {i}; revoke it first to change it"
        ));
    }
    let grant = Grant {
        new_key,
        scope: parse_pubkey(SAGE_PROGRAM)?,
        expire_time: expire_time(&s.days.get_untracked(), now)?,
        permissions: grant_permissions(s)?,
    };
    let profile_addr = s.profile_addr.get_untracked().trim().to_string();
    let ix = add_keys(wallet, parse_pubkey(&profile_addr)?, auth, &grant)?;
    let mut lines = vec![
        format!("Add key {}", encode_pubkey(&new_key)),
        format!("to profile {profile_addr}"),
        "scope SAGE".into(),
        format!(
            "permissions {}",
            perms::names(SAGE_PROGRAM, grant.permissions).join(", ")
        ),
        expiry_text(grant.expire_time),
        format!(
            "signed and paid by {} (profile key {auth})",
            encode_pubkey(&wallet)
        ),
    ];
    if perms::has(grant.permissions, DRAIN_SOL_VAULT) {
        lines.push("DRAIN_SOL_VAULT: this key can spend the profile's SOL vault".into());
    }
    if grant.expire_time == NEVER {
        lines.push("No expiration: the key works until it is revoked".into());
    }
    stage(
        s,
        "Grant a session key",
        lines,
        ix,
        Change::Grant(new_key),
        profile_addr,
    )
    .await
}

async fn prepare_revoke(s: State, index: u16, expected: Pubkey) -> Result<(), String> {
    s.pending.set(None);
    let wallet = s.wallet_key()?;
    let profile = load_profile(s).await?;
    let auth = authority_index(&profile, &wallet, REMOVE_KEYS, now_secs())?;
    if index < profile.auth_key_count || index == auth {
        return Err(format!(
            "key {index} is an auth key; this page does not remove those"
        ));
    }
    let key = key_at(&profile, index, &expected)?;
    let scope = encode_pubkey(&key.scope);
    let profile_addr = s.profile_addr.get_untracked().trim().to_string();
    let ix = remove_keys(wallet, parse_pubkey(&profile_addr)?, auth, index)?;
    let lines = vec![
        format!("Remove key {index}: {}", encode_pubkey(&key.key)),
        format!("from profile {profile_addr}"),
        format!(
            "scope {}, permissions {}",
            perms::scope_label(&scope),
            perms::names(&scope, key.permissions).join(", ")
        ),
        format!(
            "signed and paid by {} (profile key {auth})",
            encode_pubkey(&wallet)
        ),
        "Keys after it may move down an index: re-check indexes the agent uses.".into(),
    ];
    stage(
        s,
        "Revoke a key",
        lines,
        ix,
        Change::Revoke(index, expected),
        profile_addr,
    )
    .await
}

/// Simulates the instruction and shows it for review.
async fn stage(
    s: State,
    title: &str,
    lines: Vec<String>,
    ix: Instruction,
    change: Change,
    profile: String,
) -> Result<(), String> {
    let url = checked_url(s).await?;
    let payer = s.wallet_key()?;
    let message = compile_message(
        payer,
        std::slice::from_ref(&ix),
        rpc::latest_blockhash(&url).await?,
    )?;
    let tx = STANDARD.encode(unsigned_transaction(&message)?);
    let logs = rpc::simulate(&url, &tx).await?;
    s.pending.set(Some(Pending {
        title: title.into(),
        lines,
        payer,
        profile,
        rpc_url: url,
        ix,
        change,
        logs,
    }));
    Ok(())
}

async fn sign_and_send(s: State) -> Result<(), String> {
    let p = s.pending.get_untracked().ok_or("nothing to sign")?;
    if s.wallet_key()? != p.payer || s.profile_addr.get_untracked().trim() != p.profile {
        s.pending.set(None);
        return Err("the wallet or profile changed since review; review again".into());
    }
    let url = checked_url(s).await?;
    if url != p.rpc_url {
        s.pending.set(None);
        return Err("the RPC URL changed since review; review again".into());
    }
    if let Change::Revoke(index, key) = p.change
        && let Err(e) = key_at(&load_profile(s).await?, index, &key)
    {
        s.pending.set(None);
        return Err(e);
    }
    let message = compile_message(
        p.payer,
        std::slice::from_ref(&p.ix),
        rpc::latest_blockhash(&url).await?,
    )?;
    let signed = bridge::sign_transaction(&unsigned_transaction(&message)?).await?;
    let signature = check_signed(&signed, &message)?;
    let sent = rpc::send(&url, &STANDARD.encode(&signed)).await?;
    if sent != signature {
        return Err(format!(
            "the RPC returned signature {sent}, expected {signature}"
        ));
    }
    s.pending.set(None);
    s.say(Line::Link(
        format!("sent {signature}"),
        format!("{EXPLORER}/tx/{signature}"),
    ));
    rpc::confirm(&url, &signature).await?;
    let profile = load_profile(s).await?;
    s.say(Line::Info(format!("{} confirmed", p.title)));
    match p.change {
        Change::Grant(key) => {
            if let Some(i) = profile.keys.iter().position(|k| k.key == key) {
                s.say(Line::Info(format!(
                    "{} is profile key {i}: tell the agent to pass keyIndex {i} to forge-mcp's build_* methods \
                     (the signer config has no index; its optional profile = \"{}\" is informational)",
                    encode_pubkey(&key),
                    p.profile
                )));
            }
        }
        Change::Revoke(_, key) => match profile.keys.iter().position(|k| k.key == key) {
            None => s.say(Line::Info(format!(
                "{} is no longer on the profile",
                encode_pubkey(&key)
            ))),
            Some(i) => s.say(Line::Error(format!(
                "{} is still on the profile, as key {i}: check the transaction and revoke it again if needed",
                encode_pubkey(&key)
            ))),
        },
    }
    Ok(())
}

#[component]
pub fn App() -> impl IntoView {
    let s = State::new();
    s.wallets.set(bridge::wallet_names());
    // Wallets that load after the page register late.
    spawn_local(async move {
        bridge::sleep(500).await;
        s.wallets.set(bridge::wallet_names());
    });
    view! {
        <header>
            <h1>"sa-forge-signer grant"</h1>
            <p class="muted">
                "Grant or revoke a SAGE session key with the profile authority's own wallet. Z.ink testnet only."
            </p>
        </header>
        <main>
            <Network s=s />
            <Wallet s=s />
            <ProfileKeys s=s />
            <GrantForm s=s />
            <Review s=s />
            <Log s=s />
        </main>
    }
}

#[component]
fn Network(s: State) -> impl IntoView {
    view! {
        <section>
            <h2>"Network"</h2>
            <div class="row">
                <input
                    type="url"
                    aria-label="RPC URL"
                    prop:value=move || s.rpc_url.get()
                    on:input=move |ev| {
                        // A new URL is unchecked, and a review made against the old one is void.
                        s.rpc_url.set(event_target_value(&ev));
                        s.network.set(None);
                        s.pending.set(None);
                    }
                />
                <button disabled=move || s.busy.get() on:click=move |_| s.run(check_network(s))>
                    "Check"
                </button>
            </div>
            {move || match s.network.get() {
                None => view! { <p class="muted">"Not checked yet."</p> }.into_any(),
                Some(Ok(())) => view! { <p class="ok">"Z.ink testnet"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="err">{e}</p> }.into_any(),
            }}
        </section>
    }
}

#[component]
fn Wallet(s: State) -> impl IntoView {
    let disconnect = move |_| {
        s.run(async move {
            s.wallet.set(None);
            s.pending.set(None);
            bridge::disconnect().await
        });
    };
    view! {
        <section>
            <h2>"Wallet"</h2>
            {move || match s.wallet.get() {
                Some(address) => view! {
                    <div class="row">
                        <code>{address}</code>
                        <button disabled=move || s.busy.get() on:click=disconnect>"Disconnect"</button>
                    </div>
                }.into_any(),
                None => view! {
                    <div class="row wrap">
                        {move || s.wallets.get().into_iter().map(|name| {
                            let label = name.clone();
                            view! {
                                <button
                                    disabled=move || s.busy.get()
                                    on:click=move |_| s.run(connect(s, name.clone()))
                                >{label}</button>
                            }
                        }).collect_view()}
                        <button class="quiet" on:click=move |_| s.wallets.set(bridge::wallet_names())>
                            "Refresh"
                        </button>
                    </div>
                    <p class="muted">
                        "Wallets that support Wallet Standard (Phantom, Backpack, Solflare) appear here. "
                        "A Ledger signs through the wallet it is connected to; Player Profile instructions "
                        "need blind signing enabled in its Solana app."
                    </p>
                }.into_any(),
            }}
        </section>
    }
}

#[component]
fn ProfileKeys(s: State) -> impl IntoView {
    let load = move |_| {
        s.run(async move { load_profile(s).await.map(|_| ()) });
    };
    view! {
        <section>
            <h2>"Profile"</h2>
            <div class="row">
                <input type="text" bind:value=s.profile_addr placeholder="Profile address" />
                <button disabled=move || s.busy.get() on:click=load>"Load"</button>
            </div>
            {move || {
                let found = s.candidates.get();
                (found.len() > 1).then(|| view! {
                    <div class="row wrap">
                        <span class="muted">"Profiles for this wallet:"</span>
                        {found.into_iter().map(|address| {
                            let label = address.clone();
                            view! {
                                <button class="quiet" on:click=move |_| {
                                    s.profile_addr.set(address.clone());
                                    s.run(async move { load_profile(s).await.map(|_| ()) });
                                }>{label}</button>
                            }
                        }).collect_view()}
                    </div>
                })
            }}
            {move || s.profile.get().map(|p| key_table(s, &p))}
        </section>
    }
}

fn key_table(s: State, p: &Profile) -> impl IntoView + use<> {
    let now = now_secs();
    let auth_count = p.auth_key_count;
    let rows = p
        .keys
        .iter()
        .zip(0u16..)
        .map(|(k, i)| {
            let scope = encode_pubkey(&k.scope);
            let expiry = match k.expire_time {
                t if t < 0 => "never".to_string(),
                t if k.expired(now) => format!("expired {}", iso_time(t)),
                t => iso_time(t),
            };
            let removable = i >= auth_count;
            let key = k.key;
            view! {
                <tr class:expired=k.expired(now)>
                    <td>{i}</td>
                    <td><code>{encode_pubkey(&k.key)}</code></td>
                    <td>{perms::scope_label(&scope).to_string()}</td>
                    <td class="perms">{perms::names(&scope, k.permissions).join(", ")}</td>
                    <td>{expiry}</td>
                    <td>
                        {removable.then(|| view! {
                            <button
                                class="danger"
                                disabled=move || s.busy.get() || s.wallet.get().is_none()
                                on:click=move |_| s.run(prepare_revoke(s, i, key))
                            >"Revoke"</button>
                        })}
                    </td>
                </tr>
            }
        })
        .collect_view();
    view! {
        <table>
            <thead>
                <tr><th>"#"</th><th>"Key"</th><th>"Scope"</th><th>"Permissions"</th><th>"Expires"</th><th></th></tr>
            </thead>
            <tbody>{rows}</tbody>
        </table>
    }
}

#[component]
fn GrantForm(s: State) -> impl IntoView {
    let presets = PRESETS
        .iter()
        .map(|p| {
            let m = perms::mask(p.bits);
            view! { <button class="quiet" on:click=move |_| s.bits.set(m)>{p.name}</button> }
        })
        .collect_view();
    let boxes = SAGE
        .iter()
        .filter(|b| b.player)
        .map(|b| {
            let m = perms::mask(&[b.index]);
            view! {
                <label class="check">
                    <input
                        type="checkbox"
                        prop:checked=move || s.bits.get() & m != 0
                        on:change=move |_| s.bits.update(|bits| *bits ^= m)
                    />
                    {b.name}
                </label>
            }
        })
        .collect_view();
    let options = EXPIRY_DAYS
        .iter()
        .map(|d| {
            let label = if *d == NEVER {
                "No expiration".to_string()
            } else {
                format!("{d} days")
            };
            let value = d.to_string();
            let current = value.clone();
            view! {
                <option value=value selected=move || s.days.with(|v| *v == current)>{label}</option>
            }
        })
        .collect_view();
    view! {
        <section>
            <h2>"Grant a session key"</h2>
            <label>"Session key (from " <code>"sa-forge-signer key show"</code> ")"
                <input type="text" bind:value=s.new_key placeholder="Public key" />
            </label>
            <div class="row wrap">
                <span class="muted">"Presets:"</span>
                {presets}
                <button class="quiet" on:click=move |_| s.bits.set(0)>"None"</button>
            </div>
            <div class="grid">{boxes}</div>
            <label class="check warn">
                <input type="checkbox" bind:checked=s.drain />
                "DRAIN_SOL_VAULT: needed to create accounts (fleets, crafting, claim stakes), "
                "and lets the key spend the profile's SOL vault"
            </label>
            <label>"Expiration"</label>
            <div class="row">
                <select bind:value=s.days>{options}</select>
                <span class:muted=move || s.days.get() != NEVER.to_string()
                      class:warn=move || s.days.get() == NEVER.to_string()>
                    {move || match expire_time(&s.days.get(), now_secs()) {
                        Ok(NEVER) => "The key never expires: it works until revoked".to_string(),
                        Ok(t) => format!("The key will expire on {}", iso_time(t)),
                        Err(e) => e,
                    }}
                </span>
            </div>
            <button
                disabled=move || s.busy.get() || s.wallet.get().is_none()
                on:click=move |_| s.run(prepare_grant(s))
            >"Review"</button>
        </section>
    }
}

#[component]
fn Review(s: State) -> impl IntoView {
    move || {
        s.pending.get().map(|p| {
            view! {
                <section class="review">
                    <h2>{p.title.clone()}</h2>
                    <ul>{p.lines.iter().map(|l| view! { <li>{l.clone()}</li> }).collect_view()}</ul>
                    <details>
                        <summary>"Simulation passed: logs"</summary>
                        <pre>{p.logs.join("\n")}</pre>
                    </details>
                    <p class="muted">
                        "Your wallet shows the transaction next. Check it signs with the wallet above and calls "
                        <code>{profile::PLAYER_PROFILE_PROGRAM}</code> "."
                    </p>
                    <div class="row">
                        <button disabled=move || s.busy.get() on:click=move |_| s.run(sign_and_send(s))>
                            "Sign with wallet and send"
                        </button>
                        <button class="quiet" on:click=move |_| s.pending.set(None)>"Cancel"</button>
                    </div>
                </section>
            }
        })
    }
}

#[component]
fn Log(s: State) -> impl IntoView {
    view! {
        <section>
            <h2>"Log"</h2>
            <ul class="log">
                {move || s.log.get().into_iter().rev().map(|line| match line {
                    Line::Info(t) => view! { <li>{t}</li> }.into_any(),
                    Line::Error(t) => view! { <li class="err">{t}</li> }.into_any(),
                    Line::Link(t, href) => view! {
                        <li><a href=href target="_blank" rel="noopener noreferrer">{t}</a></li>
                    }.into_any(),
                }).collect_view()}
            </ul>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expire_time_from_a_choice() {
        assert_eq!(expire_time("-1", 1_000), Ok(-1));
        assert_eq!(expire_time("7", 1_000), Ok(1_000 + 7 * DAY_SECS));
        assert_eq!(expire_time(" 90 ", 0), Ok(90 * DAY_SECS));
        assert!(expire_time("365", 0).is_err());
        assert!(expire_time("8", 0).is_err());
        assert!(expire_time("0", 0).is_err());
        assert!(expire_time("", 0).is_err());
        assert!(expire_time("90", i64::MAX).is_err());
    }
}
