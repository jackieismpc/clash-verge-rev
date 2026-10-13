use super::{CmdResult, proxy_aware_coded_error};
use crate::{
    constants::timing,
    core::{
        CoreManager,
        manager::RunningMode,
        service::{SERVICE_MANAGER, ServiceStatus, request_runtime_provider_sync},
    },
};

#[derive(Debug, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ServiceInstallOutcome {
    Installed,
}

async fn execute_service_operation_sync(status: ServiceStatus, error_code: &str) -> CmdResult<ServiceInstallOutcome> {
    let manager = CoreManager::global();
    let result = {
        let _lifecycle = manager.lifecycle_lock.lock().await;
        if matches!(
            &status,
            ServiceStatus::ReinstallRequired | ServiceStatus::ForceReinstallRequired
        ) {
            manager
                .controlled_stop_core_inner()
                .await
                .map_err(|error| proxy_aware_coded_error(&error, error_code))?;
        }
        SERVICE_MANAGER.handle_service_status(status).await
    };

    result
        .map(|()| ServiceInstallOutcome::Installed)
        .map_err(|error| proxy_aware_coded_error(&error, error_code))
}

#[tauri::command]
#[specta::specta]
pub async fn install_service() -> CmdResult<ServiceInstallOutcome> {
    execute_service_operation_sync(ServiceStatus::InstallRequired, "SERVICE_INSTALL_FAILED").await
}

#[tauri::command]
#[specta::specta]
pub async fn uninstall_service() -> CmdResult {
    CoreManager::global()
        .uninstall_service_and_start_sidecar()
        .await
        .map_err(|error| proxy_aware_coded_error(&error, "SERVICE_UNINSTALL_FAILED"))
}

#[tauri::command]
#[specta::specta]
pub async fn reinstall_service() -> CmdResult<ServiceInstallOutcome> {
    execute_service_operation_sync(ServiceStatus::ReinstallRequired, "SERVICE_REINSTALL_FAILED").await
}

#[tauri::command]
#[specta::specta]
pub async fn repair_service() -> CmdResult<ServiceInstallOutcome> {
    execute_service_operation_sync(ServiceStatus::ForceReinstallRequired, "SERVICE_REPAIR_FAILED").await
}

#[tauri::command]
#[specta::specta]
pub async fn continue_with_sidecar() -> CmdResult {
    crate::core::CoreManager::global()
        .continue_with_sidecar(None)
        .await
        .map_err(|error| proxy_aware_coded_error(&error, "SERVICE_SIDECAR_FAILED"))
}

#[tauri::command]
#[specta::specta]
pub fn get_sidecar_failure() -> crate::core::notification::SidecarFailureSnapshot {
    crate::core::notification::sidecar_failure_snapshot()
}

#[tauri::command]
#[specta::specta]
pub fn take_service_fallback_notice() -> Option<crate::core::service::ServiceFallbackNotice> {
    crate::core::service::take_service_fallback_notice()
}

#[tauri::command]
#[specta::specta]
pub fn get_core_startup_error() -> Option<crate::core::manager::CoreFailure> {
    crate::core::CoreManager::global().get_startup_error()
}

#[tauri::command]
#[specta::specta]
pub fn take_service_repair_notice() -> bool {
    crate::core::service::take_service_repair_notice()
}

#[tauri::command]
#[specta::specta]
pub fn take_service_owner_notice() -> Option<String> {
    crate::core::service::take_service_owner_notice()
}

#[tauri::command]
#[specta::specta]
pub fn sync_runtime_providers() {
    if matches!(*CoreManager::global().get_running_mode(), RunningMode::Service) {
        request_runtime_provider_sync(timing::RUNTIME_PROVIDER_SETTLE);
    }
}
