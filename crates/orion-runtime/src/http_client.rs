use crate::{Connection, ExitError};
use reqwest::{blocking::Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct Health {
    name: String,
    api_version: u32,
    instance_id: String,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_wire_fixture_is_readable_by_the_native_client() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../contracts/api-v1.json")).unwrap();
        let health: Health = serde_json::from_value(fixture["health"].clone()).unwrap();
        let connection: Connection = serde_json::from_value(fixture["connection"].clone()).unwrap();
        let tasks: Vec<Task> = serde_json::from_value(fixture["tasks"].clone()).unwrap();
        assert_eq!(health.instance_id, connection.instance_id);
        assert_eq!(health.api_version, 1);
        assert!(health
            .capabilities
            .iter()
            .any(|value| value == "graceful_shutdown"));
        assert_eq!(tasks.len(), 5);
        assert_eq!(tasks[0].files, 2);
    }
}

#[derive(Deserialize)]
pub(crate) struct Task {
    pub(crate) status: String,
    pub(crate) files: u64,
}

pub(crate) enum ShutdownResult {
    Accepted,
    Unreachable,
}
pub(crate) struct LocalClient {
    client: Client,
}
impl LocalClient {
    pub(crate) fn new() -> Result<Self, String> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
    pub(crate) fn health(&self, connection: &Connection) -> Result<(), String> {
        let response = self
            .client
            .get(format!("{}/api/v1/health", connection.url))
            .bearer_auth(&connection.token)
            .send()
            .map_err(|_| "后台未响应。".to_string())?;
        let health: Health = response
            .error_for_status()
            .map_err(|_| "后台认证失败，请重新连接。".to_string())?
            .json()
            .map_err(|_| "后台返回了无效的状态。".to_string())?;
        if health.name != "orion-server"
            || health.api_version != 1
            || health.instance_id != connection.instance_id
            || !health.capabilities.iter().any(|c| c == "graceful_shutdown")
        {
            return Err("后台版本或实例不匹配，请退出旧版后台后重试。".into());
        }
        Ok(())
    }

    pub(crate) fn tasks(&self, connection: &Connection) -> Result<Vec<Task>, String> {
        self.client
            .get(format!("{}/api/v1/tasks", connection.url))
            .bearer_auth(&connection.token)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json())
            .map_err(|_| "后台已断开。".into())
    }

    pub(crate) fn shutdown(
        &self,
        connection: &Connection,
        cancel_active: bool,
    ) -> Result<ShutdownResult, ExitError> {
        match self.client.post(format!("{}/api/v1/server/shutdown", connection.url)).bearer_auth(&connection.token)
            .json(&serde_json::json!({"instance_id": connection.instance_id, "cancel_active": cancel_active})).send() {
            Ok(response) if response.status() == StatusCode::ACCEPTED => Ok(ShutdownResult::Accepted),
            Ok(response) if response.status() == StatusCode::CONFLICT => {
                let body: serde_json::Value = response.json().unwrap_or_default();
                if body["code"] == "scan_in_progress" { Err(ExitError::ActiveScan) }
                else { Err(ExitError::Failed("后台实例已变化，请重新连接后退出。".into())) }
            }
            Ok(_) => Err(ExitError::Failed("后台拒绝退出请求，请确认版本和连接状态。".into())),
            Err(error) if error.is_connect() => Ok(ShutdownResult::Unreachable),
            Err(_) => Err(ExitError::Failed("退出请求未得到确认，请稍后重试。".into())),
        }
    }

    pub(crate) fn gone(&self, connection: &Connection) -> bool {
        match self
            .client
            .get(format!("{}/api/v1/health", connection.url))
            .bearer_auth(&connection.token)
            .send()
        {
            Err(error) if error.is_connect() => true,
            Ok(response) if response.status() == StatusCode::UNAUTHORIZED => true,
            _ => false,
        }
    }
}
