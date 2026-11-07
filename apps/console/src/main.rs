//! sa-forge-signer-console: a read-only desktop view of an sa-forge-signer. It reads the audit log and
//! the config, polls `serve`'s `/health`, and never writes, signs or approves anything.

mod app;
mod config;
mod log;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use clap::Parser;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Signer config [default: `$SA_FORGE_SIGNER_CONFIG`, else `~/.config/sa-forge-signer/config.toml`]
    #[arg(long)]
    config: Option<PathBuf>,
    /// The signer's state directory (holds `audit.jsonl`) [default: the config's `state_dir`]
    #[arg(long)]
    state_dir: Option<PathBuf>,
    /// Base URL of `serve` for health checks [default: from the config's `[serve] listen`]
    #[arg(long)]
    url: Option<String>,
}

pub enum Health {
    Up,
    Down(String),
}

const HEALTH_EVERY: Duration = Duration::from_secs(3);

/// Polls `/health` on its own thread so a stalled service never blocks the UI.
fn spawn_health(url: String, ctx: eframe::egui::Context) -> mpsc::Receiver<Health> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(2)))
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let health = format!("{}/health", url.trim_end_matches('/'));
        loop {
            let h = match agent.get(&health).call() {
                Ok(_) => Health::Up,
                Err(e) => Health::Down(e.to_string()),
            };
            if tx.send(h).is_err() {
                break;
            }
            ctx.request_repaint();
            std::thread::sleep(HEALTH_EVERY);
        }
    });
    rx
}

/// `0.0.0.0` and `[::]` listen everywhere; reach them on loopback.
fn url_from_listen(listen: &str) -> String {
    let addr = listen
        .strip_prefix("0.0.0.0:")
        .map(|port| format!("127.0.0.1:{port}"))
        .or_else(|| {
            listen
                .strip_prefix("[::]:")
                .map(|port| format!("[::1]:{port}"))
        })
        .unwrap_or_else(|| listen.to_owned());
    format!("http://{addr}")
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.or_else(config::default_path);
    let (cfg, config_error) = match config_path.as_deref().map(config::Config::load) {
        Some(Ok(c)) => (c, None),
        Some(Err(e)) => (config::Config::default(), Some(format!("{e:#}"))),
        None => (
            config::Config::default(),
            Some("no config path (HOME is not set)".to_owned()),
        ),
    };
    let state_dir = cli.state_dir.or_else(|| cfg.state_dir.clone());
    let url = cli
        .url
        .or_else(|| cfg.listen.as_deref().map(url_from_listen));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("sa-forge-signer-console")
            .with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "sa-forge-signer-console",
        options,
        Box::new(move |cc| {
            let health = url.clone().map(|u| spawn_health(u, cc.egui_ctx.clone()));
            Ok(Box::new(app::App::new(app::Sources {
                config: cfg,
                config_path,
                config_error,
                state_dir,
                url,
                health,
            })))
        }),
    )
    .map_err(|e| anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_to_url() {
        assert_eq!(url_from_listen("127.0.0.1:8790"), "http://127.0.0.1:8790");
        assert_eq!(url_from_listen("0.0.0.0:9"), "http://127.0.0.1:9");
        assert_eq!(url_from_listen("[::]:9"), "http://[::1]:9");
    }
}
