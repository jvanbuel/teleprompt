//! Who a server on loopback answers. Binding to 127.0.0.1 keeps other
//! machines out, but not other web pages: a browser lets any site send a
//! request to loopback, and open a WebSocket to it. So a request must name
//! this machine (a name that resolved here only after a page loaded, DNS
//! rebinding, does not), and one a page sends must come from this server's
//! own page. A native app sends no `Origin`, and is answered.

/// Why a request with these `Host` and `Origin` headers is refused, if it
/// is.
pub fn refused(host: Option<&str>, origin: Option<&str>) -> Option<&'static str> {
    let Some(host) = host else {
        return Some("a request names its host");
    };
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    if !["127.0.0.1", "localhost"]
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n))
    {
        return Some("this server answers only to this machine's own names");
    }
    match origin {
        Some(origin) if !origin.eq_ignore_ascii_case(&format!("http://{host}")) => {
            Some("this server answers only its own page")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::refused;

    #[test]
    fn this_machine_and_its_own_page_are_answered() {
        assert_eq!(refused(Some("localhost"), None), None);
        assert_eq!(refused(Some("127.0.0.1:7879"), None), None);
        let own = Some("http://127.0.0.1:7879");
        assert_eq!(refused(Some("127.0.0.1:7879"), own), None);
    }

    #[test]
    fn another_name_or_page_is_refused() {
        assert!(refused(None, None).is_some());
        assert!(refused(Some("rebound.example:7879"), None).is_some());
        assert!(refused(Some("localhost.example"), None).is_some());
        let host = Some("127.0.0.1:7879");
        assert!(refused(host, Some("https://example.com")).is_some());
        assert!(refused(host, Some("null")).is_some());
        // Another server on this machine is another page.
        assert!(refused(host, Some("http://127.0.0.1:8000")).is_some());
    }
}
