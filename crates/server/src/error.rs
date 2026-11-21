use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error(
        "the lease on this task is gone (expired, released or re-claimed); stop work and report"
    )]
    LeaseLost,
    #[error("agent already holds its maximum number of leases")]
    MaxLeases,
    #[error("authentication required")]
    Unauthorized,
    #[error("{0}")]
    Insecure(String),
    #[error("database error: {0}")]
    Db(sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        if let Some(db) = e.as_database_error() {
            match db.code().as_deref() {
                Some("23505") => {
                    return Error::Conflict(format!("already exists ({})", db.message()));
                }
                Some("23503") => {
                    return Error::Invalid(format!("reference not found ({})", db.message()));
                }
                Some("23514") => {
                    return Error::Invalid(format!("invalid value ({})", db.message()));
                }
                _ => {}
            }
        }
        Error::Db(e)
    }
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotFound(_) => "not_found",
            Error::Forbidden(_) => "forbidden",
            Error::Invalid(_) => "invalid",
            Error::Conflict(_) => "conflict",
            Error::LeaseLost => "lease_lost",
            Error::MaxLeases => "max_leases",
            Error::Unauthorized => "unauthorized",
            Error::Insecure(_) => "insecure_transport",
            Error::Db(_) | Error::Internal(_) => "internal",
        }
    }
    pub fn status(&self) -> StatusCode {
        match self {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::Conflict(_) | Error::LeaseLost | Error::MaxLeases => StatusCode::CONFLICT,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Insecure(_) => StatusCode::FORBIDDEN,
            Error::Db(_) | Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
    pub fn body(&self) -> serde_json::Value {
        let msg = match self {
            Error::Db(e) => {
                tracing::error!("db error: {e}");
                "internal error".to_string()
            }
            Error::Internal(m) => {
                tracing::error!("internal error: {m}");
                "internal error".to_string()
            }
            other => other.to_string(),
        };
        json!({ "error": self.code(), "message": msg })
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut resp = (self.status(), Json(self.body())).into_response();
        if matches!(self, Error::Unauthorized) {
            resp.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                "Bearer realm=\"corntchap\"".parse().unwrap(),
            );
        }
        resp
    }
}

pub fn invalid<T>(m: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(m.into()))
}
