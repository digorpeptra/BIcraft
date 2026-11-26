//! The server argument of `Sql.Database` (`host`, `host,port`, `host\instance`, `tcp:host,port`,
//! `[::1],1433`) and the SQL Server Browser lookup of a named instance's port ([MC-SQLR]:
//! `CLNT_UCAST_INST` over UDP 1434, answered by `SVR_RESP`).

use std::net::UdpSocket;
use std::time::Duration;

/// Where to connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// A DNS name or an IP address (without brackets).
    pub host: String,
    pub port: Option<u16>,
    pub instance: Option<String>,
}

/// The default port of the default instance.
pub const DEFAULT_PORT: u16 = 1433;
/// The SQL Server Browser's port.
const BROWSER_PORT: u16 = 1434;

fn bad(text: &str, why: &str) -> String {
    format!("`{text}` is not a SQL Server name ({why}); expected host, host,port or host\\instance")
}

/// Parses the server text.
pub fn parse(text: &str) -> Result<Server, String> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    let t = if lower.starts_with("tcp:") {
        t.get(4..).unwrap_or_default().trim()
    } else if ["np:", "lpc:", "admin:"].iter().any(|p| lower.starts_with(p)) || lower.starts_with("(localdb)")
    {
        return Err(format!(
            "`{text}`: bicraft connects to SQL Server over TCP only (not named pipes, shared memory or LocalDB)"
        ));
    } else {
        t
    };
    let (rest, port) = match t.rsplit_once(',') {
        Some((h, p)) => {
            let p = p.trim();
            let port: u16 = p.parse().map_err(|_| bad(text, "the port is not a number"))?;
            if port == 0 {
                return Err(bad(text, "port 0"));
            }
            (h.trim(), Some(port))
        }
        None => (t, None),
    };
    let (host, instance) = match rest.split_once('\\') {
        Some((h, i)) => (h.trim(), Some(i.trim())),
        None => (rest, None),
    };
    let host = match host {
        "." | "(local)" => "localhost",
        h => h,
    };
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let ok_host = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'));
    if !ok_host {
        return Err(bad(text, "the host name"));
    }
    let instance = match instance {
        None => None,
        Some(i)
            if !i.is_empty()
                && i.len() <= 16
                && i.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'#')) =>
        {
            Some(i.to_string())
        }
        Some(_) => return Err(bad(text, "the instance name")),
    };
    Ok(Server {
        host: host.to_ascii_lowercase(),
        port,
        instance,
    })
}

/// The TCP port of `SVR_RESP`'s instance list (`…;tcp;1433;…`).
pub fn browser_port(answer: &[u8]) -> Option<u16> {
    let [0x05, l0, l1, rest @ ..] = answer else {
        return None;
    };
    let len = usize::from(u16::from_le_bytes([*l0, *l1]));
    let text = String::from_utf8_lossy(rest.get(..len.min(rest.len()))?);
    let parts: Vec<&str> = text.split(';').collect();
    let at = parts.iter().position(|p| p.eq_ignore_ascii_case("tcp"))?;
    parts.get(at + 1)?.trim().parse().ok().filter(|p| *p != 0)
}

/// Asks the SQL Server Browser of `host` for the port of `instance`.
pub fn instance_port(host: &str, instance: &str, timeout: Duration) -> Result<u16, String> {
    instance_port_at(host, BROWSER_PORT, instance, timeout)
}

pub(crate) fn instance_port_at(
    host: &str,
    browser: u16,
    instance: &str,
    timeout: Duration,
) -> Result<u16, String> {
    let fail = |why: String| {
        format!(
            "SQL Server: the port of instance `{instance}` on `{host}` is unknown ({why}); give it as host,port"
        )
    };
    let sock = UdpSocket::bind(if host.contains(':') { "[::]:0" } else { "0.0.0.0:0" })
        .map_err(|e| fail(e.to_string()))?;
    sock.set_read_timeout(Some(timeout))
        .map_err(|e| fail(e.to_string()))?;
    sock.connect((host, browser)).map_err(|e| fail(e.to_string()))?;
    let mut ask = vec![0x04];
    ask.extend_from_slice(instance.as_bytes());
    ask.push(0);
    sock.send(&ask).map_err(|e| fail(e.to_string()))?;
    let mut buf = vec![0u8; 4096];
    let n = sock
        .recv(&mut buf)
        .map_err(|e| fail(format!("no answer from the SQL Server Browser: {e}")))?;
    browser_port(buf.get(..n).unwrap_or_default())
        .ok_or_else(|| fail("the browser's answer has no TCP port".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(host: &str, port: Option<u16>, instance: Option<&str>) -> Server {
        Server {
            host: host.into(),
            port,
            instance: instance.map(str::to_string),
        }
    }

    #[test]
    fn server_names() {
        assert_eq!(parse("db.example.com").unwrap(), s("db.example.com", None, None));
        assert_eq!(
            parse(" tcp:DB.Example.com , 1500 ").unwrap(),
            s("db.example.com", Some(1500), None)
        );
        assert_eq!(
            parse(r"srv\SQLEXPRESS").unwrap(),
            s("srv", None, Some("SQLEXPRESS"))
        );
        assert_eq!(
            parse(r".\SQLEXPRESS").unwrap(),
            s("localhost", None, Some("SQLEXPRESS"))
        );
        assert_eq!(parse("(local)").unwrap(), s("localhost", None, None));
        assert_eq!(parse("[::1],1433").unwrap(), s("::1", Some(1433), None));
        assert_eq!(parse("127.0.0.1,5000").unwrap(), s("127.0.0.1", Some(5000), None));
        for bad in [
            "",
            "a b",
            "h,0",
            "h,99999",
            "h,x",
            r"h\",
            r"h\in;st",
            "np:\\\\h\\pipe",
            "(localdb)\\v11",
            "a/b",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn browser_answers() {
        let text = b"ServerName;H;InstanceName;SQLEXPRESS;IsClustered;No;Version;16.0.1000.6;tcp;51234;;";
        let mut a = vec![0x05];
        a.extend_from_slice(&u16::try_from(text.len()).unwrap().to_le_bytes());
        a.extend_from_slice(text);
        assert_eq!(browser_port(&a), Some(51234));
        assert_eq!(browser_port(&[0x05, 0xFF, 0xFF, b't']), None);
        assert_eq!(browser_port(&[]), None);
        // a local browser
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"\x04SQLEXPRESS\0");
            server.send_to(&a, from).unwrap();
        });
        assert_eq!(
            instance_port_at("127.0.0.1", port, "SQLEXPRESS", Duration::from_secs(5)).unwrap(),
            51234
        );
        t.join().unwrap();
    }
}
