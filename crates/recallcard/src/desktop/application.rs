//! 新应用服务的桌面边界；只核验会话/范围，不复制业务流程。
use super::{check_scope, DesktopSession};
use crate::application::{
    background_memory::{MemoryRuntime, MemoryRuntimeStatus},
    AppError, AppResult, ErrorCode, ImportRequest, ImportResult, ImportService, JobStatus,
};
use std::path::PathBuf;
fn access_error() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "当前资料库会话或范围已失效",
        "重新打开当前资料库后重试",
    )
}
impl DesktopSession {
    fn application_service(&self, session_id: &str, scope: &str) -> AppResult<ImportService> {
        check_scope(scope).map_err(|_| access_error())?;
        ImportService::new(self.vault(session_id).map_err(|_| access_error())?)
    }
    pub fn application_import_sources(
        &self,
        session_id: &str,
        scope: &str,
        request_id: &str,
        paths: &[PathBuf],
    ) -> AppResult<JobStatus> {
        let service = self.application_service(session_id, scope)?;
        let job = service.submit(ImportRequest {
            request_id: request_id.into(),
            paths: paths.to_vec(),
            scope: scope.into(),
            format: "auto".into(),
        })?;
        service.start_worker()?;
        Ok(job)
    }
    pub fn application_jobs(&self, session_id: &str, scope: &str) -> AppResult<Vec<JobStatus>> {
        self.application_service(session_id, scope)?.list(scope)
    }
    pub fn application_job(
        &self,
        session_id: &str,
        scope: &str,
        job_id: &str,
    ) -> AppResult<JobStatus> {
        self.application_service(session_id, scope)?
            .status(job_id, scope)
    }
    pub fn application_import_result(
        &self,
        session_id: &str,
        scope: &str,
        job_id: &str,
    ) -> AppResult<ImportResult> {
        self.application_service(session_id, scope)?
            .result(job_id, scope)
    }
    pub fn application_job_pause(
        &self,
        session_id: &str,
        scope: &str,
        job_id: &str,
    ) -> AppResult<JobStatus> {
        self.application_service(session_id, scope)?
            .pause(job_id, scope)
    }
    pub fn application_job_resume(
        &self,
        session_id: &str,
        scope: &str,
        job_id: &str,
    ) -> AppResult<JobStatus> {
        let service = self.application_service(session_id, scope)?;
        let job = service.resume(job_id, scope)?;
        service.start_worker()?;
        Ok(job)
    }
    pub fn memory_runtime_status(
        &self,
        session_id: &str,
        scope: &str,
    ) -> AppResult<MemoryRuntimeStatus> {
        check_scope(scope).map_err(|_| access_error())?;
        MemoryRuntime::new(self.vault(session_id).map_err(|_| access_error())?)
            .status_for_scope(scope)
    }
}
