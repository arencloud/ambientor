use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum JwtError {
    #[error("token error: {0}")]
    Token(#[from] jsonwebtoken::errors::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub username: String,
    pub roles: Vec<String>,
    pub exp: i64,
}

pub struct JwtService {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl JwtService {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        }
    }

    pub fn issue(
        &self,
        user_id: Uuid,
        username: &str,
        roles: Vec<String>,
    ) -> Result<String, JwtError> {
        let claims = Claims {
            sub: user_id,
            username: username.to_string(),
            roles,
            exp: (Utc::now() + Duration::hours(8)).timestamp(),
        };
        encode(&Header::default(), &claims, &self.encoding).map_err(JwtError::from)
    }

    pub fn verify(&self, token: &str) -> Result<Claims, JwtError> {
        let data = decode::<Claims>(token, &self.decoding, &Validation::default())?;
        Ok(data.claims)
    }
}
