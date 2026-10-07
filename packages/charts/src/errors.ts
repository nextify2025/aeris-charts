/** Stable machine-readable categories for public API failures. */
export type AerisChartsErrorCode =
  | "disposed"
  | "invalid_handle"
  | "stale_handle"
  | "invalid_data"
  | "invalid_options"
  | "unsupported_operation"
  | "unsupported"
  | "reentrant_call"
  | "serialization_error"
  | "persistence_version_error"
  | "extension_error"
  | "renderer_platform_error"
  | "resource_limit";

/** Public error thrown for predictable chart, handle, validation, persistence, and platform failures. */
export class AerisChartsError extends Error {
  override readonly name = "AerisChartsError";

  constructor(
    readonly code: AerisChartsErrorCode,
    message: string,
  ) {
    super(`aeris_charts: ${message}`);
  }
}
