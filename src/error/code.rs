use axum::http::StatusCode;

/// Machine-readable error metadata attached to every [`crate::error::Error`] variant.
///
/// Implementors provide a stable, uppercase code string, an appropriate HTTP
/// status code for use in API responses, and an actionable suggestion to help
/// operators diagnose and resolve the issue.
pub trait ErrorCode {
    /// A stable, `SCREAMING_SNAKE_CASE` machine-readable identifier for this error.
    fn code(&self) -> &'static str;

    /// The HTTP status code most appropriate for this error class.
    fn http_status(&self) -> StatusCode;

    /// A short, human-readable action the operator can take to resolve this error.
    fn suggestion(&self) -> String;
}
