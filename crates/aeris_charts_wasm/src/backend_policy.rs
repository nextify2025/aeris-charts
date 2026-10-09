//! Platform-neutral classification for WebGPU surface failures.
//!
//! Keeping this policy outside the browser-only chart module makes recovery semantics testable on
//! the host while the actual surface/canvas transition remains in the WASM adapter.

use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct BackendStatus {
    pub requested_backend: &'static str,
    pub active_backend: &'static str,
    pub stage: &'static str,
    pub reason: &'static str,
    pub secure_context: Option<bool>,
    pub navigator_gpu: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BackendStartupFailure {
    stage: &'static str,
    reason: &'static str,
    detail: String,
}

impl BackendStartupFailure {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn is_adapter_unavailable(&self) -> bool {
        self.reason == "adapter_unavailable"
    }

    pub(crate) fn adapter(detail: String) -> Self {
        Self {
            stage: "adapter_acquisition",
            reason: "adapter_unavailable",
            detail,
        }
    }

    pub(crate) fn device(detail: String) -> Self {
        Self {
            stage: "device_acquisition",
            reason: "device_unavailable",
            detail,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn surface(detail: String) -> Self {
        Self {
            stage: "surface_configuration",
            reason: "surface_unavailable",
            detail,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn initialization(detail: String) -> Self {
        Self {
            stage: "initialization",
            reason: "webgpu_initialization_failed",
            detail,
        }
    }
}

impl BackendStatus {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn canvas2d_requested(
        secure_context: Option<bool>,
        navigator_gpu: Option<bool>,
    ) -> Self {
        Self {
            requested_backend: "canvas2d",
            active_backend: "canvas2d",
            stage: "backend_selection",
            reason: "canvas2d_requested",
            secure_context,
            navigator_gpu,
            detail: None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn webgpu_ready(secure_context: Option<bool>, navigator_gpu: Option<bool>) -> Self {
        Self {
            requested_backend: "auto",
            active_backend: "webgpu",
            stage: "ready",
            reason: "webgpu_ready",
            secure_context,
            navigator_gpu,
            detail: None,
        }
    }

    pub(crate) fn startup_fallback(
        failure: BackendStartupFailure,
        secure_context: Option<bool>,
        navigator_gpu: Option<bool>,
    ) -> Self {
        Self {
            requested_backend: "auto",
            active_backend: "canvas2d",
            stage: failure.stage,
            reason: failure.reason,
            secure_context,
            navigator_gpu,
            detail: Some(failure.detail),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn runtime_fallback(&mut self, reason: &'static str, detail: String) {
        self.active_backend = "canvas2d";
        self.stage = "runtime";
        self.reason = reason;
        self.detail = Some(detail);
    }
}

#[derive(Default)]
pub(crate) struct BackendWarningDeduplicator {
    seen: HashSet<(&'static str, &'static str)>,
}

impl BackendWarningDeduplicator {
    pub(crate) fn should_warn(&mut self, status: &BackendStatus) -> bool {
        self.seen.insert((status.stage, status.reason))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceErrorAction {
    Reconfigure,
    SkipFrame,
    Fallback,
}

pub(crate) fn surface_error_action(error: &wgpu::CurrentSurfaceTexture) -> SurfaceErrorAction {
    match error {
        wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
            SurfaceErrorAction::Reconfigure
        }
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
            SurfaceErrorAction::SkipFrame
        }
        wgpu::CurrentSurfaceTexture::Validation => SurfaceErrorAction::Fallback,
        wgpu::CurrentSurfaceTexture::Success(_) | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
            unreachable!("successful acquisitions are handled before error classification")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackendStartupFailure, BackendStatus, BackendWarningDeduplicator, SurfaceErrorAction,
        surface_error_action,
    };

    #[test]
    fn startup_failure_stage_is_independent_of_platform_detail() {
        let status = BackendStatus::startup_fallback(
            BackendStartupFailure::device("webgpu found no adapters in nested text".into()),
            Some(true),
            Some(true),
        );
        assert_eq!(status.stage, "device_acquisition");
        assert_eq!(status.reason, "device_unavailable");
    }

    #[test]
    fn warning_deduplication_does_not_merge_per_chart_status() {
        let first = BackendStatus::startup_fallback(
            BackendStartupFailure::adapter("webgpu found no adapters".into()),
            Some(true),
            Some(true),
        );
        let second = first.clone();
        let mut warnings = BackendWarningDeduplicator::default();

        assert!(warnings.should_warn(&first));
        assert!(!warnings.should_warn(&second));
        assert_eq!(first, second);
        assert_eq!(second.active_backend, "canvas2d");
    }

    #[test]
    fn recoverable_surface_errors_reconfigure_once() {
        assert_eq!(
            surface_error_action(&wgpu::CurrentSurfaceTexture::Lost),
            SurfaceErrorAction::Reconfigure
        );
        assert_eq!(
            surface_error_action(&wgpu::CurrentSurfaceTexture::Outdated),
            SurfaceErrorAction::Reconfigure
        );
    }

    #[test]
    fn timeout_skips_only_the_current_frame() {
        assert_eq!(
            surface_error_action(&wgpu::CurrentSurfaceTexture::Timeout),
            SurfaceErrorAction::SkipFrame
        );
    }

    #[test]
    fn occlusion_skips_and_validation_falls_back() {
        assert_eq!(
            surface_error_action(&wgpu::CurrentSurfaceTexture::Occluded),
            SurfaceErrorAction::SkipFrame
        );
        assert_eq!(
            surface_error_action(&wgpu::CurrentSurfaceTexture::Validation),
            SurfaceErrorAction::Fallback
        );
    }
}
