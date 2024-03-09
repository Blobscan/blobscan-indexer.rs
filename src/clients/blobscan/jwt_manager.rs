use std::sync::{Arc, Mutex};

use chrono::{Duration, TimeDelta, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    exp: usize,
}

#[derive(Debug, Clone)]
pub struct JWTManager {
    // Use the Arc<Mutex<>> pattern for interior mutability
    token: Arc<Mutex<Option<String>>>,
    expiration_date: Arc<Mutex<Option<chrono::DateTime<Utc>>>>,

    secret_key: String,
    refresh_interval: Duration,
    safety_margin: Duration,
}

pub struct Config {
    pub secret_key: String,
    pub refresh_interval: Duration,
    pub safety_magin: Option<Duration>,
}

impl JWTManager {
    pub fn new(config: Config) -> Self {
        Self {
            token: Arc::new(Mutex::new(None)),
            expiration_date: Arc::new(Mutex::new(None)),
            secret_key: config.secret_key,
            refresh_interval: config.refresh_interval,
            safety_margin: match config.safety_magin {
                Some(safety_margin) => safety_margin,
                None => TimeDelta::try_minutes(1).unwrap(),
            },
        }
    }

    pub fn get_token(&self) -> Result<String, anyhow::Error> {
        let mut token_guard = self.token.lock().unwrap();
        let mut expr_guard = self.expiration_date.lock().unwrap();

        match *token_guard {
            Some(ref token) => {
                let now = Utc::now() - self.safety_margin;
                let expiration_date = expr_guard.ok_or(anyhow::anyhow!(
                    "JWT expiration date not set. This should not happen"
                ))?;

                if now > expiration_date {
                    debug!(
                        target = "jwt_manager",
                        expiration_date = expiration_date.to_string(),
                        "JWT expired. Refreshing token"
                    );

                    let (token, expiration_date) = self.create_token()?;

                    *token_guard = Some(token.clone());
                    *expr_guard = Some(expiration_date);

                    return Ok(token);
                }
                Ok(token.clone())
            }
            None => {
                let (token, expiration_date) = self.create_token()?;

                *token_guard = Some(token.clone());
                *expr_guard = Some(expiration_date);

                Ok(token)
            }
        }
    }

    fn create_token(&self) -> Result<(String, chrono::DateTime<Utc>), anyhow::Error> {
        let encoding_key = EncodingKey::from_secret(self.secret_key.as_ref());
        let expiration_date = chrono::Utc::now() + self.refresh_interval;
        let claims = Claims {
            exp: expiration_date.timestamp() as usize,
        };
        let header = Header::new(Algorithm::HS512);

        match encode(&header, &claims, &encoding_key) {
            Err(error) => {
                error!(target = "jwt_manager", ?error, "Failed to create JWT");

                Err(error.into())
            }
            Ok(t) => {
                debug!(target = "jwt_manager", "JWT created");

                Ok((t, expiration_date))
            }
        }
    }
}
