//! Opt-in tracing on stderr (stdout carries the JSON report). `SA_FORGE_SIGNER_LOG` sets the level
//! (`error`, `warn`, `info`, `debug`, `trace`; unset or `off`: none) and `SA_FORGE_SIGNER_LOG_FORMAT`
//! the format (`compact`, the default, or `json`). Each span logs its duration when it closes.
//!
//! Spans and events carry key names, payload hashes, RPC method names, counts and outcomes only:
//! never key material, partial signers, tokens or their hashes, the RPC URL (it may hold an API
//! key), or caller-supplied text (`summary`, `intent`).

use anyhow::{Result, bail};
use tracing::Subscriber;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Compact,
    Json,
}

/// The level and format from the two variables' values; `None` when tracing is off.
pub fn settings(
    level: Option<&str>,
    format: Option<&str>,
) -> Result<Option<(LevelFilter, Format)>> {
    let level = match level.map(|l| l.trim().to_ascii_lowercase()).as_deref() {
        None | Some("" | "off") => return Ok(None),
        Some("error") => LevelFilter::ERROR,
        Some("warn") => LevelFilter::WARN,
        Some("info") => LevelFilter::INFO,
        Some("debug") => LevelFilter::DEBUG,
        Some("trace") => LevelFilter::TRACE,
        Some(other) => {
            bail!("SA_FORGE_SIGNER_LOG={other:?}: use error, warn, info, debug, trace or off")
        }
    };
    let format = match format.map(|f| f.trim().to_ascii_lowercase()).as_deref() {
        None | Some("" | "compact") => Format::Compact,
        Some("json") => Format::Json,
        Some(other) => bail!("SA_FORGE_SIGNER_LOG_FORMAT={other:?}: use compact or json"),
    };
    Ok(Some((level, format)))
}

/// A subscriber writing to `writer`.
pub fn subscriber<W>(
    level: LevelFilter,
    format: Format,
    writer: W,
) -> Box<dyn Subscriber + Send + Sync>
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let builder = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_max_level(level)
        .with_span_events(FmtSpan::CLOSE)
        .with_target(false)
        .with_ansi(false);
    match format {
        Format::Compact => Box::new(builder.compact().finish()),
        Format::Json => Box::new(builder.json().with_current_span(true).finish()),
    }
}

static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether `init` installed a subscriber.
pub fn active() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Installs the subscriber the environment asks for, if any.
pub fn init() -> Result<()> {
    let level = std::env::var("SA_FORGE_SIGNER_LOG").ok();
    let format = std::env::var("SA_FORGE_SIGNER_LOG_FORMAT").ok();
    let Some((level, format)) = settings(level.as_deref(), format.as_deref())? else {
        return Ok(());
    };
    if tracing::subscriber::set_global_default(subscriber(level, format, std::io::stderr)).is_err()
    {
        bail!("a tracing subscriber is already installed");
    }
    ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A buffer tests read back.
    #[derive(Clone, Default)]
    struct Captured(pub Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Captured {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    thread_local! {
        static CAPTURE: std::cell::RefCell<Option<Captured>> = const { std::cell::RefCell::new(None) };
    }

    /// Writes to this thread's capture, if one is running; elsewhere, nowhere.
    struct ThisThread;

    impl std::io::Write for ThisThread {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            CAPTURE.with(|c| {
                if let Some(out) = c.borrow_mut().as_mut() {
                    out.write_all(buf)?;
                }
                Ok(buf.len())
            })
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'w> MakeWriter<'w> for ThisThread {
        type Writer = Self;
        fn make_writer(&'w self) -> Self::Writer {
            Self
        }
    }

    /// Runs `f` and returns what it traced at debug level, as JSON lines. A subscriber set for one
    /// test thread misses spans that other test threads met first, so one global subscriber serves
    /// every test and keeps only the capturing thread's output.
    pub fn capture<R>(f: impl FnOnce() -> R) -> (R, String) {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            let _ = tracing::subscriber::set_global_default(subscriber(
                LevelFilter::DEBUG,
                Format::Json,
                ThisThread,
            ));
        });
        let out = Captured::default();
        CAPTURE.with(|c| *c.borrow_mut() = Some(out.clone()));
        let r = f();
        CAPTURE.with(|c| *c.borrow_mut() = None);
        (r, out.text())
    }

    #[test]
    fn settings_come_from_the_two_variables() {
        assert_eq!(settings(None, None).unwrap(), None);
        assert_eq!(settings(Some("off"), Some("json")).unwrap(), None);
        assert_eq!(
            settings(Some("info"), None).unwrap(),
            Some((LevelFilter::INFO, Format::Compact))
        );
        assert_eq!(
            settings(Some(" DEBUG "), Some("json")).unwrap(),
            Some((LevelFilter::DEBUG, Format::Json))
        );
        assert!(
            settings(Some("loud"), None).is_err(),
            "an unknown level is an error, not silence"
        );
        assert!(settings(Some("info"), Some("xml")).is_err());
    }

    #[test]
    fn a_span_logs_its_duration_when_it_closes() {
        let ((), text) = capture(|| {
            let _s = tracing::info_span!("phase", key = "k").entered();
        });
        let line: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(line["span"]["name"], "phase", "{text}");
        assert_eq!(line["span"]["key"], "k");
        assert!(line["fields"]["time.busy"].is_string(), "{text}");
    }
}
