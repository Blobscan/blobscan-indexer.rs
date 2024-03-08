use std::{fmt::Display, str::FromStr};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum NumericOrTextCode {
    String(String),
    Number(usize),
}
/// API Error response
#[derive(Deserialize, Debug, Clone)]
pub struct ErrorResponse {
    /// Error code
    pub code: NumericOrTextCode,
    /// Error message
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Reqwest Error
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    /// API Error
    #[error("API usage error: {0}")]
    ApiError(ErrorResponse),

    /// Other Error
    #[error(transparent)]
    Other(#[from] anyhow::Error),

    /// Url Parsing Error
    #[error("{0}")]
    UrlParse(#[from] url::ParseError),

    /// Serde Json deser Error
    #[error("{0}")]
    SerdeError(#[from] serde_json::Error),
}

/// API Response
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ClientResponse<T> {
    /// Error
    Error(ErrorResponse),
    /// Success w/ value
    Success(T),
    /// Empty Success
    EmptySuccess,
}

pub type ClientResult<T> = Result<T, ClientError>;

impl<T> ClientResponse<T> {
    pub(crate) fn into_client_result(self) -> ClientResult<Option<T>> {
        match self {
            ClientResponse::Error(e) => Err(e.into()),
            ClientResponse::Success(t) => Ok(Some(t)),
            ClientResponse::EmptySuccess => Ok(None),
        }
    }

    /// True if the response is an API error
    pub fn is_err(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

impl<T> FromStr for ClientResponse<T>
where
    T: serde::de::DeserializeOwned,
{
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(ClientResponse::EmptySuccess);
        }
        serde_json::from_str(s)
    }
}

impl From<ErrorResponse> for ClientError {
    fn from(err: ErrorResponse) -> Self {
        Self::ApiError(err)
    }
}

impl Display for NumericOrTextCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(s) => f.write_str(s.to_string().as_ref()),
            Self::Number(n) => f.write_str(n.to_string().as_ref()),
        }
    }
}
impl Display for ErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "Code: {}, Message: \"{}\"",
            self.code,
            self.message.as_deref().unwrap_or(""),
        ))
    }
}
