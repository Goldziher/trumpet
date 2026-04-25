//! Axum response integration for [`crate::error::Error`].

use axum::Json;
use axum::response::{IntoResponse, Response};

use crate::error::{Error, ErrorCode as _};

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.http_status();
        let body = self.to_json_body(false);
        (status, Json(body)).into_response()
    }
}
