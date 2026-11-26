//! Trusted identity headers set by an auth proxy (oauth2-proxy, Pomerium, Authelia, Traefik
//! forward-auth…), accepted only from configured proxy addresses.

use std::net::IpAddr;

/// An address range: `10.0.0.0/8`, `fd00::/8`, or one address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Result<Cidr, String> {
        let s = s.trim();
        let bad = || format!("`{s}` is not an IP address or CIDR range");
        let (ip, prefix) = match s.split_once('/') {
            Some((ip, p)) => (ip, Some(p.parse::<u8>().map_err(|_| bad())?)),
            None => (s, None),
        };
        let addr = canonical(ip.parse::<IpAddr>().map_err(|_| bad())?);
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = prefix.unwrap_or(max);
        if prefix > max {
            return Err(bad());
        }
        Ok(Cidr { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, canonical(ip)) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX.checked_shl(32 - u32::from(self.prefix)).unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX.checked_shl(128 - u32::from(self.prefix)).unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// IPv4-mapped IPv6 addresses (`::ffff:10.0.0.1`, as a dual-stack socket reports IPv4 peers)
/// compare as IPv4.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// The trusted-header mode.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Peers allowed to send the identity headers. Required, never empty.
    pub trusted: Vec<Cidr>,
    /// The user header (default `X-Forwarded-User`, oauth2-proxy's; Pomerium:
    /// `X-Pomerium-Claim-Email`, Authelia: `Remote-User`).
    pub user_header: String,
    /// Optional e-mail header (default `X-Forwarded-Email`).
    pub email_header: Option<String>,
    /// Optional groups header (default `X-Forwarded-Groups`).
    pub groups_header: Option<String>,
    /// Separator of the groups header (default `,`).
    pub groups_separator: String,
}

impl ProxyConfig {
    pub fn new(trusted: Vec<Cidr>) -> Self {
        Self {
            trusted,
            user_header: "X-Forwarded-User".into(),
            email_header: Some("X-Forwarded-Email".into()),
            groups_header: Some("X-Forwarded-Groups".into()),
            groups_separator: ",".into(),
        }
    }

    pub fn check(&self) -> Result<(), String> {
        if self.trusted.is_empty() {
            return Err("the trusted-header mode needs at least one trusted proxy address".into());
        }
        for h in self.header_names() {
            if h.is_empty()
                || !h
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(format!("`{h}` is not a valid header name"));
            }
            if h.eq_ignore_ascii_case("authorization") || h.eq_ignore_ascii_case("host") {
                return Err(format!("`{h}` cannot carry the identity"));
            }
        }
        if self.groups_separator.is_empty() {
            return Err("the groups separator cannot be empty".into());
        }
        Ok(())
    }

    /// Every identity header name.
    pub fn header_names(&self) -> Vec<&str> {
        let mut v = vec![self.user_header.as_str()];
        v.extend(self.email_header.as_deref());
        v.extend(self.groups_header.as_deref());
        v
    }

    /// Whether `peer` may send the identity headers.
    pub fn trusts(&self, peer: IpAddr) -> bool {
        self.trusted.iter().any(|c| c.contains(peer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn ranges() {
        let c = Cidr::parse("10.0.0.0/8").unwrap();
        assert!(c.contains(ip("10.1.2.3")));
        assert!(c.contains(ip("::ffff:10.1.2.3")));
        assert!(!c.contains(ip("11.0.0.1")));
        assert!(!c.contains(ip("::1")));
        let one = Cidr::parse("192.168.1.7").unwrap();
        assert!(one.contains(ip("192.168.1.7")));
        assert!(!one.contains(ip("192.168.1.8")));
        let all = Cidr::parse("0.0.0.0/0").unwrap();
        assert!(all.contains(ip("8.8.8.8")));
        let v6 = Cidr::parse("fd00::/8").unwrap();
        assert!(v6.contains(ip("fd12::1")));
        assert!(!v6.contains(ip("fe80::1")));
        assert!(Cidr::parse("::1").unwrap().contains(ip("::1")));
        assert!(Cidr::parse("::ffff:127.0.0.1").unwrap().contains(ip("127.0.0.1")));
        for bad in ["", "10.0.0.0/33", "::/129", "x", "10.0.0.0/", "10.0.0.0/-1"] {
            assert!(Cidr::parse(bad).is_err(), "{bad}");
        }
    }
}
