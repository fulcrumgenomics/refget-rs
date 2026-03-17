pub mod seqcol;
pub mod sequences;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use refget_model::ErrorResponse;

/// Create a JSON error response with the given status code and message.
pub fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    let body = ErrorResponse { status_code: status.as_u16(), message: message.into() };
    (status, axum::Json(body)).into_response()
}
