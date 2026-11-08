//! sa-forge-signer-grant: a local page where the profile authority's own wallet adds or removes a
//! session key on a SAGE profile. It holds no keys; the wallet signs.

#![expect(
    clippy::future_not_send,
    reason = "wasm is single-threaded; futures over JS promises are never Send"
)]

mod app;
mod bridge;
mod perms;
mod profile;
mod rpc;
mod tx;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}
