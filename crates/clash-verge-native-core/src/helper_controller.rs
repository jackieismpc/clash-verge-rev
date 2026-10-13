//! Privileged controller firewall. Raw root-engine reload APIs never reach the user socket.
use anyhow::{Result, bail};
use serde_json::Value;

pub fn allowed_request(method: &str, target: &str, body: &[u8], websocket: bool) -> Result<()> {
    if target.len() > 8192 || !target.starts_with('/') || target.contains('#') || target.contains('\\') {
        bail!("invalid controller request target");
    }
    let path = target.split('?').next().unwrap_or_default();
    let mut decoded = Vec::new();
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                bail!("invalid percent encoding");
            }
            let high = (bytes[index + 1] as char)
                .to_digit(16)
                .ok_or_else(|| anyhow::anyhow!("invalid percent encoding"))?;
            let low = (bytes[index + 2] as char)
                .to_digit(16)
                .ok_or_else(|| anyhow::anyhow!("invalid percent encoding"))?;
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = std::str::from_utf8(&decoded)?;
    if decoded.contains('\\')
        || decoded.bytes().any(|byte| byte < 0x20)
        || decoded.split('/').any(|segment| matches!(segment, "." | ".."))
        || decoded.contains("//")
    {
        bail!("ambiguous decoded controller request path");
    }
    if path.contains("/../") || path.contains("/./") || path.contains("//") {
        bail!("ambiguous controller request path");
    }
    if websocket {
        if method != "GET" || !matches!(path, "/logs" | "/traffic" | "/memory" | "/connections") {
            bail!("unsupported privileged controller stream");
        }
        if !body.is_empty() {
            bail!("controller stream cannot carry a request body");
        }
        return Ok(());
    }
    match method {
        "GET"
            if matches!(
                path,
                "/version"
                    | "/configs"
                    | "/proxies"
                    | "/connections"
                    | "/providers/proxies"
                    | "/providers/rules"
                    | "/rules"
            ) || path.starts_with("/proxies/")
                || path.starts_with("/providers/proxies/")
                || path.starts_with("/providers/rules/") =>
        {
            if !body.is_empty() {
                bail!("controller read cannot carry a request body");
            }
        }
        "DELETE" if path == "/connections" || path.starts_with("/connections/") => {
            if !body.is_empty() {
                bail!("connection close cannot carry a request body");
            }
        }
        "PUT" if path.starts_with("/proxies/") => {
            let value: Value = serde_json::from_slice(body)?;
            let object = value
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("proxy selection requires object"))?;
            if object.len() != 1 || object.get("name").and_then(Value::as_str).is_none() {
                bail!("only named proxy selection is permitted");
            }
        }
        "PUT" if path.starts_with("/providers/rules/") => {
            if !body.is_empty() && body != b"{}" {
                bail!("rule-provider update cannot carry configuration");
            }
        }
        "PATCH" if path == "/configs" => {
            let value: Value = serde_json::from_slice(body)?;
            let object = value
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("runtime patch requires object"))?;
            if object.is_empty() {
                bail!("empty runtime patch");
            }
            for (key, value) in object {
                match (key.as_str(), value) {
                    ("mode", Value::String(value)) if ["rule", "global", "direct"].contains(&value.as_str()) => (),
                    ("log-level", Value::String(value))
                        if ["silent", "error", "warning", "info", "debug"].contains(&value.as_str()) => {}
                    _ => bail!("privileged controller forbids runtime field {key}"),
                }
            }
        }
        _ => bail!("privileged controller operation requires a validated helper transaction"),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_privileged_reload_and_tun_bypass() {
        assert!(allowed_request("PUT", "/configs", br#"{"path":"/tmp/user-config"}"#, false).is_err());
        assert!(allowed_request("PATCH", "/configs", br#"{"tun":{"enable":true}}"#, false).is_err());
        assert!(allowed_request("GET", "/proxies/../configs", b"", false).is_err());
        assert!(allowed_request("PUT", "/proxies/%2e%2e/configs", br#"{"name":"node"}"#, false).is_err());
        assert!(allowed_request("PUT", "/providers/proxies/foo", b"{}", false).is_err());
        assert!(allowed_request("PUT", "/proxies/p", br#"{"name":"node","path":"/etc/passwd"}"#, false).is_err());
        assert!(allowed_request("GET", "/configs", b"", true).is_err());
    }
    #[test]
    fn permits_read_select_stream_and_mode() {
        assert!(allowed_request("GET", "/proxies/node%2Fname/delay?timeout=3000", b"", false).is_ok());
        assert!(allowed_request("PUT", "/proxies/group", br#"{"name":"node"}"#, false).is_ok());
        assert!(allowed_request("PATCH", "/configs", br#"{"mode":"rule"}"#, false).is_ok());
        assert!(allowed_request("GET", "/logs?level=info", b"", true).is_ok());
    }
}
