//! URL rules shared by the config, the service and the stdio proxy.

/// Whether `url` is https, or http to a loopback host: signed transactions and any API key in
/// the URL never cross a network in clear.
pub fn secure(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit('@').next().unwrap_or_default();
    let host = host_name(host_port);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The host part of `host[:port]` or `[v6]:port`.
pub fn host_name(authority: &str) -> &str {
    let authority = authority.split('/').next().unwrap_or_default();
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or_default();
    }
    authority.split(':').next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_must_be_https_unless_loopback() {
        for ok in [
            "https://rpc1.z.ink",
            "https://rpc.example/?api-key=x",
            "http://127.0.0.1:8899",
            "http://127.9.9.9",
            "http://localhost:8899/",
            "http://[::1]:8899",
        ] {
            assert!(secure(ok), "{ok}");
        }
        for bad in [
            "http://rpc1.z.ink",
            "http://10.0.0.5:8899",
            "http://127.0.0.1.evil.example",
            "http://127.0.0.1@evil.example",
            "http://localhost.evil.example",
            "ftp://127.0.0.1",
            "rpc1.z.ink",
        ] {
            assert!(!secure(bad), "{bad}");
        }
    }

    #[test]
    fn host_names() {
        assert_eq!(host_name("127.0.0.1:8790"), "127.0.0.1");
        assert_eq!(host_name("localhost"), "localhost");
        assert_eq!(host_name("[::1]:8790"), "::1");
        assert_eq!(host_name("evil.example:80/x"), "evil.example");
    }
}
