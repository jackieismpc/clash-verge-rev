//! Offline conversion of supported share links to mihomo proxy mappings.
use anyhow::{Result, bail};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, net::IpAddr};

const INVALID: &str = "Invalid proxy URI";
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|v| u8::from_str_radix(v, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}
fn b64(s: &str) -> String {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    for engine in [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(&compact)
            && let Ok(text) = String::from_utf8(bytes)
            && text.chars().all(|c| !c.is_control() || "\t\r\n".contains(c))
        {
            return text;
        }
    }
    s.to_owned()
}
fn query(s: &str) -> BTreeMap<String, String> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.trim().replace('_', "-"), decode(v))
        })
        .collect()
}
fn port(s: &str) -> Result<u16> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        bail!(INVALID);
    }
    let p = s.parse::<u16>().map_err(|_| anyhow::anyhow!(INVALID))?;
    if p == 0 {
        bail!(INVALID);
    }
    Ok(p)
}
fn endpoint(s: &str, required: bool) -> Result<(String, u16)> {
    let s = s.trim_end_matches('/');
    let (host, p) = if s.starts_with('[') {
        let end = s.find(']').ok_or_else(|| anyhow::anyhow!(INVALID))?;
        let host = &s[1..end];
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            bail!(INVALID);
        }
        let rest = &s[end + 1..];
        (
            host,
            if rest.is_empty() {
                None
            } else {
                Some(rest.strip_prefix(':').ok_or_else(|| anyhow::anyhow!(INVALID))?)
            },
        )
    } else {
        if s.matches(':').count() > 1 {
            bail!(INVALID);
        }
        s.rsplit_once(':').map(|(h, p)| (h, Some(p))).unwrap_or((s, None))
    };
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || "/?#@".contains(c)) {
        bail!(INVALID);
    }
    let p = match p {
        Some(p) => port(p)?,
        None if required => bail!(INVALID),
        None => 443,
    };
    Ok((host.to_owned(), p))
}
fn string(m: &mut Map<String, Value>, key: &str, value: Option<&String>) {
    if let Some(v) = value
        && !v.is_empty()
    {
        m.insert(key.into(), json!(v));
    }
}
fn flag(m: &mut Map<String, Value>, key: &str, value: Option<&String>) {
    if let Some(v) = value {
        m.insert(
            key.into(),
            json!(v.is_empty() || v == "1" || v.eq_ignore_ascii_case("true")),
        );
    }
}
fn number(m: &mut Map<String, Value>, key: &str, value: Option<&String>) {
    if let Some(n) = value.and_then(|v| v.parse::<i64>().ok()) {
        m.insert(key.into(), json!(n));
    }
}
fn list(m: &mut Map<String, Value>, key: &str, value: Option<&String>) {
    if let Some(v) = value {
        let a: Vec<_> = v.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if !a.is_empty() {
            m.insert(key.into(), json!(a));
        }
    }
}
fn transport(m: &mut Map<String, Value>, q: &BTreeMap<String, String>, default_tcp: bool) {
    let raw = q
        .get("net")
        .or_else(|| q.get("type"))
        .map(String::as_str)
        .unwrap_or("tcp");
    let network = if q.get("headerType").is_some_and(|v| v == "http") {
        "http"
    } else {
        match raw {
            "websocket" | "sw" | "httpupgrade" => "ws",
            "ws" | "grpc" | "h2" | "http" | "xhttp" => raw,
            _ => "tcp",
        }
    };
    if network == "tcp" && !default_tcp {
        return;
    }
    m.insert("network".into(), json!(network));
    let host = q.get("host").or_else(|| q.get("obfsParam"));
    let path = q.get("path");
    let mut opts = Map::new();
    match network {
        "ws" => {
            string(&mut opts, "path", path);
            if let Some(h) = host {
                let headers = serde_json::from_str::<Value>(h)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({"Host":h}));
                opts.insert("headers".into(), headers);
            }
            if raw == "httpupgrade" {
                opts.insert("v2ray-http-upgrade".into(), json!(true));
                opts.insert("v2ray-http-upgrade-fast-open".into(), json!(true));
            }
        }
        "grpc" => string(&mut opts, "grpc-service-name", q.get("serviceName").or(path)),
        "h2" | "xhttp" => {
            string(&mut opts, "host", host);
            string(&mut opts, "path", path);
            if network == "xhttp" {
                string(&mut opts, "mode", q.get("mode"));
            }
        }
        "http" => {
            if let Some(p) = path {
                opts.insert("path".into(), json!([p]));
            }
            if let Some(h) = host {
                opts.insert("headers".into(), json!({"Host":[h]}));
            }
        }
        _ => {}
    }
    if !opts.is_empty() {
        m.insert(format!("{network}-opts"), Value::Object(opts));
    }
    if m.get("tls") == Some(&json!(true)) && !m.contains_key("servername") {
        string(m, "servername", host);
    }
}
fn cipher(s: &str) -> &str {
    let s = if s == "chacha20-poly1305" {
        "chacha20-ietf-poly1305"
    } else {
        s
    };
    if s.is_empty() {
        return "none";
    }
    if [
        "none",
        "auto",
        "dummy",
        "aes-128-gcm",
        "aes-192-gcm",
        "aes-256-gcm",
        "lea-128-gcm",
        "lea-192-gcm",
        "lea-256-gcm",
        "aes-128-gcm-siv",
        "aes-256-gcm-siv",
        "2022-blake3-aes-128-gcm",
        "2022-blake3-aes-256-gcm",
        "aes-128-cfb",
        "aes-192-cfb",
        "aes-256-cfb",
        "aes-128-ctr",
        "aes-192-ctr",
        "aes-256-ctr",
        "chacha20",
        "chacha20-ietf",
        "chacha20-ietf-poly1305",
        "2022-blake3-chacha20-poly1305",
        "rabbit128-poly1305",
        "xchacha20-ietf-poly1305",
        "xchacha20",
        "aegis-128l",
        "aegis-256",
        "aez-384",
        "deoxys-ii-256-128",
        "rc4-md5",
    ]
    .contains(&s)
    {
        s
    } else {
        "auto"
    }
}

/// Errors deliberately omit the input, authentication material and server names.
pub fn parse_uri(input: &str) -> Result<Value> {
    if input.len() > 65536 {
        bail!("Proxy URI exceeds size limit");
    }
    let (scheme, rest) = input.trim().split_once("://").ok_or_else(|| anyhow::anyhow!(INVALID))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme == "vmess" {
        return vmess(rest);
    }
    if scheme == "ssr" {
        return ssr(rest);
    }
    let kind = match scheme.as_str() {
        "ss" => "ss",
        "vless" => "vless",
        "trojan" => "trojan",
        "anytls" => "anytls",
        "hy" | "hysteria" => "hysteria",
        "hy2" | "hysteria2" => "hysteria2",
        "tuic" => "tuic",
        "wg" | "wireguard" => "wireguard",
        "http" | "https" => "http",
        "socks" | "socks5" => "socks5",
        _ => bail!("Unsupported proxy URI scheme"),
    };
    let (rest, name) = rest.split_once('#').unwrap_or((rest, ""));
    let (main, qs) = rest.split_once('?').unwrap_or((rest, ""));
    let expanded = if matches!(kind, "ss" | "vless") && !main.contains('@') {
        b64(main)
    } else {
        main.to_owned()
    };
    let (auth_raw, ep) = expanded.rsplit_once('@').unwrap_or(("", &expanded));
    let (server, p) = endpoint(ep, kind == "ss" || kind == "vless")?;
    let auth = decode(auth_raw);
    if matches!(kind, "trojan" | "hysteria2" | "vless" | "tuic") && auth.is_empty() {
        bail!(INVALID);
    }
    let q = query(qs);
    if kind == "vless"
        && q.get("type").is_some_and(|v| {
            ![
                "tcp",
                "ws",
                "websocket",
                "sw",
                "httpupgrade",
                "http",
                "grpc",
                "h2",
                "xhttp",
            ]
            .contains(&v.as_str())
        })
    {
        bail!("Unsupported proxy transport");
    }
    let label = match kind {
        "ss" => "SS",
        "vless" => "VLESS",
        "trojan" => "Trojan",
        "anytls" => "AnyTLS",
        "hysteria" => "Hysteria",
        "hysteria2" => "Hysteria2",
        "tuic" => "TUIC",
        "wireguard" => "WireGuard",
        "http" => "HTTP",
        _ => "SOCKS5",
    };
    let decoded_name = decode(name);
    let name = if decoded_name.trim().is_empty() {
        q.get("remarks")
            .or_else(|| q.get("remark"))
            .cloned()
            .unwrap_or_else(|| format!("{label} {server}:{p}"))
    } else {
        decoded_name.trim().to_owned()
    };
    let mut m = json!({"type":kind,"name":name,"server":server,"port":p})
        .as_object()
        .unwrap()
        .clone();
    match kind {
        "ss" => {
            let user = b64(&auth);
            let (c, password) = user.split_once(':').ok_or_else(|| anyhow::anyhow!(INVALID))?;
            m.insert("cipher".into(), json!(cipher(c)));
            m.insert("password".into(), json!(decode(password)));
            if let Some(plugin) = q.get("plugin") {
                let mut parts = plugin.split(';');
                let plugin = parts.next().unwrap_or("");
                let opts = query(&parts.collect::<Vec<_>>().join("&"));
                let mut out = Map::new();
                match plugin {
                    "obfs-local" | "simple-obfs" => {
                        m.insert("plugin".into(), json!("obfs"));
                        string(&mut out, "mode", opts.get("obfs"));
                        string(&mut out, "host", opts.get("obfs-host"));
                    }
                    "v2ray-plugin" => {
                        m.insert("plugin".into(), json!(plugin));
                        out.insert("mode".into(), json!("websocket"));
                        string(&mut out, "host", opts.get("obfs-host").or_else(|| opts.get("host")));
                        string(&mut out, "path", opts.get("path"));
                        flag(&mut out, "tls", opts.get("tls"));
                    }
                    _ => bail!("Unsupported proxy plugin"),
                }
                m.insert("plugin-opts".into(), Value::Object(out));
            } else if let Some(v) = q.get("v2ray-plugin") {
                m.insert("plugin".into(), json!("v2ray-plugin"));
                m.insert(
                    "plugin-opts".into(),
                    serde_json::from_str(&b64(v))
                        .ok()
                        .filter(Value::is_object)
                        .unwrap_or(json!({})),
                );
            }
            flag(&mut m, "udp-over-tcp", q.get("uot"));
            flag(&mut m, "tfo", q.get("tfo"));
        }
        "http" | "socks5" => {
            if !auth.is_empty() {
                let (u, p) = auth_raw.split_once(':').unwrap_or((auth_raw, ""));
                m.insert("username".into(), json!(decode(u)));
                m.insert("password".into(), json!(decode(p)));
            }
            if scheme == "https" {
                m.insert("tls".into(), json!(true));
            } else {
                flag(&mut m, "tls", q.get("tls"));
            }
            if let Some(v) = q.get("ip-version") {
                m.insert(
                    "ip-version".into(),
                    json!(
                        if ["dual", "ipv4", "ipv6", "ipv4-prefer", "ipv6-prefer"].contains(&v.as_str()) {
                            v.as_str()
                        } else {
                            "dual"
                        }
                    ),
                );
            }
        }
        "vless" => {
            m.insert("uuid".into(), json!(auth.strip_prefix("auto:").unwrap_or(&auth)));
            if let Some(e) = q.get("encryption") {
                m.insert("encryption".into(), json!(if e == "none" { "" } else { e }));
            }
            if let Some(s) = q.get("security") {
                if s != "none" {
                    m.insert("tls".into(), json!(true));
                }
                if s == "reality" {
                    let mut o = Map::new();
                    string(&mut o, "public-key", q.get("pbk"));
                    string(&mut o, "short-id", q.get("sid"));
                    if !o.is_empty() {
                        m.insert("reality-opts".into(), Value::Object(o));
                    }
                }
            }
            if !q.contains_key("security") {
                flag(&mut m, "tls", q.get("tls"));
            }
            string(&mut m, "servername", q.get("sni").or_else(|| q.get("peer")));
            if let Some(f) = q.get("flow")
                && !f.eq_ignore_ascii_case("none")
                && !f.is_empty()
                && f.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                m.insert("flow".into(), json!(f));
            }
            string(&mut m, "client-fingerprint", q.get("fp"));
            flag(&mut m, "skip-cert-verify", q.get("allowInsecure"));
            transport(&mut m, &q, true);
        }
        "trojan" | "hysteria2" | "anytls" => {
            let password = if kind == "anytls" {
                auth.split_once(':').map(|(_, p)| p).unwrap_or(&auth)
            } else {
                &auth
            };
            if !password.is_empty() {
                m.insert("password".into(), json!(password));
            }
            string(&mut m, "sni", q.get("sni").or_else(|| q.get("peer")));
            if kind == "trojan" {
                transport(&mut m, &q, false);
                if let Some(e) = q.get("encryption") {
                    let e: Vec<_> = e.split(';').collect();
                    if e.len() == 3 {
                        m.insert("ss-opts".into(), json!({"enabled":true,"method":e[1],"password":e[2]}));
                    }
                }
            }
            if kind == "hysteria2" {
                string(&mut m, "ports", q.get("mport"));
                string(&mut m, "obfs", q.get("obfs").filter(|v| v.as_str() != "none"));
                string(&mut m, "obfs-password", q.get("obfs-password"));
                string(&mut m, "fingerprint", q.get("pinSHA256"));
                flag(&mut m, "tfo", q.get("fastopen"));
            }
            if kind == "anytls" {
                m.insert("udp".into(), json!(true));
                for k in [
                    "idle-session-check-interval",
                    "idle-session-timeout",
                    "min-idle-session",
                ] {
                    number(&mut m, k, q.get(k));
                }
                string(&mut m, "fingerprint", q.get("hpkp"));
            }
        }
        "tuic" => {
            let (uuid, password) = auth.split_once(':').ok_or_else(|| anyhow::anyhow!(INVALID))?;
            m.insert("uuid".into(), json!(uuid));
            m.insert("password".into(), json!(password));
            for k in ["token", "ip", "udp-relay-mode", "congestion-controller", "sni"] {
                string(&mut m, k, q.get(k));
            }
            for k in [
                "heartbeat-interval",
                "request-timeout",
                "max-udp-relay-packet-size",
                "max-open-streams",
            ] {
                number(&mut m, k, q.get(k));
            }
            for k in ["disable-sni", "reduce-rtt", "fast-open"] {
                flag(&mut m, k, q.get(k));
            }
        }
        "hysteria" => {
            m.insert(
                "protocol".into(),
                json!(q.get("protocol").map(String::as_str).unwrap_or("udp")),
            );
            for (a, b) in [
                ("auth", "auth-str"),
                ("mport", "ports"),
                ("obfsParam", "obfs"),
                ("upmbps", "up"),
                ("downmbps", "down"),
                ("peer", "sni"),
                ("sni", "sni"),
                ("obfs", "obfs"),
                ("ca", "ca"),
                ("ca-str", "ca-str"),
            ] {
                string(&mut m, b, q.get(a));
            }
            for k in ["recv-window", "recv-window-conn"] {
                number(&mut m, k, q.get(k));
            }
            for k in ["fast-open", "disable-mtu-discovery"] {
                flag(&mut m, k, q.get(k));
            }
        }
        "wireguard" => {
            if !auth.is_empty() {
                m.insert("private-key".into(), json!(auth));
            }
            m.insert("udp".into(), json!(true));
            for (a, b) in [
                ("publickey", "public-key"),
                ("public-key", "public-key"),
                ("pre-shared-key", "pre-shared-key"),
                ("dialer-proxy", "dialer-proxy"),
            ] {
                string(&mut m, b, q.get(a));
            }
            for k in ["dns", "allowed-ips"] {
                list(&mut m, k, q.get(k));
            }
            number(&mut m, "mtu", q.get("mtu"));
            flag(&mut m, "remote-dns-resolve", q.get("remote-dns-resolve"));
            if let Some(a) = q.get("address").or_else(|| q.get("ip")) {
                for a in a.split(',') {
                    let a = a.trim().split('/').next().unwrap_or("").trim_matches(['[', ']']);
                    if let Ok(ip) = a.parse::<IpAddr>() {
                        m.insert(if ip.is_ipv4() { "ip" } else { "ipv6" }.into(), json!(a));
                    }
                }
            }
            if let Some(r) = q.get("reserved") {
                let v: Vec<_> = r.split(',').filter_map(|s| s.trim().parse::<u8>().ok()).collect();
                if v.len() == 3 {
                    m.insert("reserved".into(), json!(v));
                }
            }
        }
        _ => unreachable!(),
    }
    list(&mut m, "alpn", q.get("alpn"));
    string(
        &mut m,
        "fingerprint",
        q.get("fingerprint")
            .or_else(|| q.get("fp").filter(|_| kind == "trojan")),
    );
    string(
        &mut m,
        "client-fingerprint",
        q.get("client-fingerprint")
            .or_else(|| q.get("fp").filter(|_| kind == "anytls")),
    );
    flag(
        &mut m,
        "skip-cert-verify",
        q.get("skip-cert-verify")
            .or_else(|| q.get("insecure"))
            .or_else(|| q.get("allow-insecure")),
    );
    flag(&mut m, "udp", q.get("udp"));
    Ok(Value::Object(m))
}
fn ssr(rest: &str) -> Result<Value> {
    let decoded = b64(rest);
    let (main, qs) = decoded.split_once("/?").unwrap_or((&decoded, ""));
    let parts: Vec<_> = main.rsplitn(6, ':').collect();
    if parts.len() != 6 {
        bail!(INVALID);
    }
    let host = parts[5].trim_matches(['[', ']']);
    let endpoint_text = if host.contains(':') {
        format!("[{host}]:{}", parts[4])
    } else {
        format!("{host}:{}", parts[4])
    };
    let (server, p) = endpoint(&endpoint_text, true)?;
    let q = query(qs);
    let mut m=json!({"type":"ssr","name":q.get("remarks").map(|v|b64(v).trim().to_owned()).unwrap_or(server.clone()),"server":server,"port":p,"protocol":parts[3],"cipher":cipher(parts[2]),"obfs":parts[1],"password":b64(parts[0])}).as_object().unwrap().clone();
    for (a, b) in [("protoparam", "protocol-param"), ("obfsparam", "obfs-param")] {
        if let Some(v) = q.get(a) {
            let v: String = b64(v).chars().filter(|c| !c.is_whitespace()).collect();
            if !v.is_empty() {
                m.insert(b.into(), json!(v));
            }
        }
    }
    Ok(Value::Object(m))
}
fn vmess(rest: &str) -> Result<Value> {
    let decoded = b64(rest);
    let mut data: Value = serde_json::from_str(&decoded).unwrap_or(Value::Null);
    if !data.is_object() {
        if decoded.contains("= vmess") || decoded.contains("=vmess") {
            return quantumult(&decoded);
        }
        let (raw, qs) = rest.split_once('?').ok_or_else(|| anyhow::anyhow!(INVALID))?;
        let decoded = b64(raw.trim_end_matches('/'));
        let (auth, ep) = decoded.rsplit_once('@').ok_or_else(|| anyhow::anyhow!(INVALID))?;
        let (c, id) = auth.split_once(':').ok_or_else(|| anyhow::anyhow!(INVALID))?;
        let (server, p) = endpoint(ep, true)?;
        data = json!({"add":server,"port":p,"id":id,"scy":c});
        for (k, v) in query(qs) {
            data[k] = json!(v);
        }
    }
    let d = data.as_object().ok_or_else(|| anyhow::anyhow!(INVALID))?;
    let get = |k: &str| -> Option<String> {
        d.get(k)
            .filter(|v| !v.is_null())
            .map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()))
    };
    let server = get("add")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!(INVALID))?
        .trim_matches(['[', ']'])
        .to_owned();
    let p = port(&get("port").ok_or_else(|| anyhow::anyhow!(INVALID))?)?;
    let endpoint_text = if server.contains(':') {
        format!("[{server}]:{p}")
    } else {
        format!("{server}:{p}")
    };
    endpoint(&endpoint_text, true)?;
    let id = get("id")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!(INVALID))?;
    let tls = get("tls").is_some_and(|v| ["tls", "true", "1"].contains(&v.as_str()));
    let mut m=json!({"type":"vmess","name":get("ps").or_else(||get("remarks")).or_else(||get("remark")).unwrap_or_else(||format!("VMess {server}:{p}")),"server":server,"port":p,"uuid":id,"cipher":cipher(&get("scy").unwrap_or("auto".into())),"tls":tls,"alterId":get("aid").or_else(||get("alterId")).and_then(|v|v.parse::<u64>().ok()).unwrap_or(0)}).as_object().unwrap().clone();
    if tls {
        string(&mut m, "servername", get("sni").as_ref());
    }
    if let Some(v) = get("verify-cert").or_else(|| get("verify_cert")) {
        m.insert("skip-cert-verify".into(), json!(!["1", "true"].contains(&v.as_str())));
    }
    let mut q = BTreeMap::new();
    for k in ["net", "type", "path", "host", "obfsParam"] {
        if let Some(v) = d.get(k) {
            let v = if let Some(a) = v.as_array() {
                a.first().and_then(Value::as_str).unwrap_or("").to_owned()
            } else {
                v.as_str().unwrap_or("").to_owned()
            };
            let v = if k == "host" || k == "obfsParam" {
                serde_json::from_str::<Value>(&v)
                    .ok()
                    .and_then(|headers| {
                        headers.get("Host").and_then(|h| {
                            h.as_str().map(str::to_owned).or_else(|| {
                                h.as_array()
                                    .and_then(|a| a.first())
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                            })
                        })
                    })
                    .unwrap_or(v)
            } else {
                v
            };
            q.insert(k.to_owned(), v);
        }
    }
    if get("obfs").is_some_and(|v| v == "websocket") {
        q.insert("net".into(), "ws".into());
    }
    transport(&mut m, &q, false);
    if m.get("network").and_then(Value::as_str) == Some("http") {
        let values = |key: &str| -> Vec<String> {
            match d.get(key) {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
                    .collect(),
                Some(Value::String(v)) if !v.trim().is_empty() => vec![v.trim().to_owned()],
                _ => Vec::new(),
            }
        };
        let hosts = values("host");
        let mut paths = values("path");
        if paths.is_empty() {
            paths.push("/".into());
        }
        let mut opts = json!({"path": paths});
        if !hosts.is_empty() {
            opts["headers"] = json!({"Host":hosts});
        }
        m.insert("http-opts".into(), opts);
    }
    Ok(Value::Object(m))
}
fn quantumult(decoded: &str) -> Result<Value> {
    let parts: Vec<_> = decoded.split(',').map(str::trim).collect();
    if parts.len() < 5 {
        bail!(INVALID);
    }
    let p = port(parts[2])?;
    let mut q = BTreeMap::new();
    for part in &parts[5..] {
        if let Some((k, v)) = part.split_once('=') {
            q.insert(k.trim().to_owned(), v.trim().trim_matches('"').to_owned());
        }
    }
    let mut m=json!({"type":"vmess","name":parts[0].split('=').next().unwrap_or("").trim(),"server":parts[1].trim_matches(['[',']']),"port":p,"cipher":cipher(parts[3]),"uuid":parts[4].trim_matches('"'),"tls":q.get("obfs").is_some_and(|v|v=="wss")}).as_object().unwrap().clone();
    flag(&mut m, "udp", q.get("udp-relay"));
    flag(&mut m, "tfo", q.get("fast-open"));
    if let Some(v) = q.get("tls-verification") {
        m.insert("skip-cert-verify".into(), json!(!["true", "1"].contains(&v.as_str())));
    }
    if let Some(o) = q.get("obfs") {
        if !["ws", "wss"].contains(&o.as_str()) {
            bail!("Unsupported proxy transport");
        }
        m.insert("network".into(), json!("ws"));
        let host = q
            .get("obfs-header")
            .and_then(|h| h.split_once("Host:").map(|(_, v)| v.trim()))
            .unwrap_or("");
        m.insert(
            "ws-opts".into(),
            json!({"path":q.get("obfs-path").map(String::as_str).unwrap_or("/"),"headers":{"Host":host}}),
        );
    }
    Ok(Value::Object(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_anonymous_examples() {
        let links = [
            "ss://YWVzLTEyOC1nY206ZXhhbXBsZQ@example.invalid:443#SS",
            "ssr://ZXhhbXBsZS5pbnZhbGlkOjQ0MzpvcmlnaW46YWVzLTEyOC1nY206cGxhaW46WlhoaGJYQnNaUQ",
            "vmess://eyJhZGQiOiJleGFtcGxlLmludmFsaWQiLCJwb3J0Ijo0NDMsImlkIjoiMDAwMDAwMDAtMDAwMC0wMDAwLTAwMDAtMDAwMDAwMDAwMDAwIn0",
            "vless://00000000-0000-0000-0000-000000000000@example.invalid:443",
            "trojan://example@example.invalid:443",
            "anytls://example@example.invalid:443",
            "hysteria://example.invalid:443?auth=example",
            "hy2://example@example.invalid:443",
            "tuic://00000000-0000-0000-0000-000000000000:example@example.invalid:443",
            "wg://example@example.invalid:443?address=192.0.2.1/32",
            "http://example.invalid:443",
            "https://example.invalid:443",
            "socks5://example.invalid:443",
        ];
        let types = [
            "ss",
            "ssr",
            "vmess",
            "vless",
            "trojan",
            "anytls",
            "hysteria",
            "hysteria2",
            "tuic",
            "wireguard",
            "http",
            "http",
            "socks5",
        ];
        for (link, expected_type) in links.into_iter().zip(types) {
            let v = parse_uri(link).expect("Anonymous protocol fixture failed");
            assert_eq!(v["type"], expected_type);
            assert_eq!(v["server"], "example.invalid");
            assert_eq!(v["port"], 443);
        }
    }
    #[test]
    fn encoded_credentials_and_vmess_http_arrays() {
        let v = parse_uri("http://user%20name:sample%3Avalue@example.invalid:80#Demo%20Proxy").unwrap();
        assert_eq!(v["type"], "http");
        assert_eq!(v["username"], "user name");
        assert_eq!(v["password"], "sample:value");
        assert_eq!(v["name"], "Demo Proxy");
        let v = parse_uri("https://user%3Aname:sample%252Fvalue@example.invalid:443?tls=false").unwrap();
        assert_eq!(v["username"], "user:name");
        assert_eq!(v["password"], "sample%2Fvalue");
        assert_eq!(v["tls"], true);
        let v = parse_uri("vless://sample@example.invalid:443?security=reality&tls=false&pbk=sample").unwrap();
        assert_eq!(v["tls"], true);
        let raw = json!({"add":"example.invalid", "port":"443", "id":"00000000-0000-0000-0000-000000000000", "net":"http", "host":["first.invalid", "second.invalid"], "path":["/a", "/b"], "tls":"tls"});
        let v = parse_uri(&format!("vmess://{}", STANDARD.encode(raw.to_string()))).unwrap();
        assert_eq!(v["http-opts"]["path"], json!(["/a", "/b"]));
        assert_eq!(
            v["http-opts"]["headers"]["Host"],
            json!(["first.invalid", "second.invalid"])
        );
        assert_eq!(v["servername"], "first.invalid");
        assert!(parse_uri("vless://example@example.invalid:443?type=unsupported").is_err());
        assert_eq!(decode("%测试"), "%测试");
    }
    #[test]
    fn strict_ports_and_redacted_errors() {
        for scheme in [
            "ss", "vless", "trojan", "anytls", "hy2", "hy", "tuic", "wg", "http", "socks5",
        ] {
            for p in ["0", "65536", "-1", "443junk", ""] {
                let link = format!("{scheme}://example:example@example.invalid:{p}");
                let e = parse_uri(&link).unwrap_err().to_string();
                assert!(!e.contains("example"));
            }
        }
    }
    #[test]
    fn ipv6_encoding_reality_and_transports() {
        let v=parse_uri("VLESS://00000000-0000-0000-0000-000000000000@[2001:db8::1]:443?security=reality&pbk=example&sid=00&type=grpc&serviceName=demo%2Fservice&sni=example.invalid#%E6%B5%8B%E8%AF%95").unwrap();
        assert_eq!(v["server"], "2001:db8::1");
        assert_eq!(v["name"], "测试");
        assert_eq!(v["tls"], true);
        assert_eq!(v["reality-opts"]["public-key"], "example");
        assert_eq!(v["grpc-opts"]["grpc-service-name"], "demo/service");
        let v = parse_uri(
            "trojan://sample%3Avalue@example.invalid?type=ws&host=front.invalid&path=%2Fdemo&skip_cert_verify=false",
        )
        .unwrap();
        assert_eq!(v["password"], "sample:value");
        assert_eq!(v["ws-opts"]["path"], "/demo");
        assert_eq!(v["skip-cert-verify"], false);
    }
    #[test]
    fn wireguard_and_plugins() {
        let v=parse_uri("wg://example@[2001:db8::1]:51820?address=192.0.2.1%2F32,2001%3Adb8%3A%3A2%2F128&reserved=1,2,3&public_key=example&remote_dns_resolve").unwrap();
        assert_eq!(v["ipv6"], "2001:db8::2");
        assert_eq!(v["reserved"], json!([1, 2, 3]));
        assert_eq!(v["remote-dns-resolve"], true);
        let v=parse_uri("ss://YWVzLTEyOC1nY206ZXhhbXBsZQ@example.invalid:443?plugin=v2ray-plugin%3Btls%3Bhost%3Dfront.invalid%3Bpath%3D%2Fdemo").unwrap();
        assert_eq!(v["plugin-opts"]["tls"], true);
    }
}
