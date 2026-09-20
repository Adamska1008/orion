use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Connection {
    pub url: String,
    pub token: String,
    pub instance_id: String,
}

pub(crate) fn read_connection(path: &Path) -> Result<Connection, String> {
    let bytes = fs::read(path).map_err(|_| "未找到后台连接信息。".to_string())?;
    let mut connection: Connection =
        serde_json::from_slice(&bytes).map_err(|_| "后台连接信息格式无效。".to_string())?;
    connection.url = validate_url(&connection.url)?;
    if connection.token.trim().is_empty() || connection.instance_id.is_empty() {
        return Err("后台连接信息不完整。".into());
    }
    Ok(connection)
}

fn validate_url(input: &str) -> Result<String, String> {
    let url = Url::parse(input).map_err(|_| "后台地址无效。".to_string())?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("后台地址必须是本机回环地址。".into());
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_urls_are_loopback_only_and_cannot_redirect_credentials() {
        assert_eq!(
            validate_url("http://127.0.0.1:43120/").unwrap(),
            "http://127.0.0.1:43120"
        );
        for value in [
            "https://example.com",
            "http://127.0.0.1.evil:80",
            "http://user@127.0.0.1:43120",
            "http://127.0.0.1:43120/?token=x",
            "http://127.0.0.1:43120/path",
        ] {
            assert!(validate_url(value).is_err(), "{value}");
        }
    }
}
