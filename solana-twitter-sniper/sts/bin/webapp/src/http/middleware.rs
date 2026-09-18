use std::sync::Arc;

use axum::{
    extract::{FromRequestParts, Request},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json, RequestPartsExt,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use jsonwebtoken::{decode, Algorithm, Validation};
use serde_json::json;

use crate::{
    models::{
        claims::{AuthError, Claims},
        state::AppState,
    },
    utils::constants::KEYS,
};

// pub async fn guard<T>(req: Request<T>, next: Next<T>) -> Result<Response, StatusCode> {
//     // let is_api_endpoint = req.uri().path().starts_with("/api");
//     // if is_api_endpoint {
//     let mut parts = req.into_parts().0;
//     let extracted_data = parts.extract::<TypedHeader<Authorization<Bearer>>>().await;
//     let bearer: Bearer = match extracted_data {
//         Ok(TypedHeader(Authorization(bearer))) => {
//             let response = next.run(req).await;
//             return Ok(response);
//         }
//         Err(_) => return Err(StatusCode::UNAUTHORIZED),
//     };

//     //     let token_data = decode::<Claims>(bearer.token(), &KEYS.decoding, &Validation::default());
//     //     let abc = match token_data {
//     //         Ok(claims) => claims,
//     //         Err(_) => return Err(StatusCode::UNAUTHORIZED),
//     //     };
//     //     let response = next.run(req).await;
//     //     // return Ok(abc.claims);

//     //     // return Ok(StatusCode::UNAUTHORIZED)
//     // }
//     // let response = next.run(req).await;
//     // Ok(response)
// }

// impl Display for Claims {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         write!(f, "Email: {}\nCompany: {}", self.sub, self.company)
//     }
// }

impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract the token from the authorization header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::InvalidToken)?;
        // Decode the user data
        let validation = Validation::new(Algorithm::EdDSA);

        let token_data = decode::<Claims>(bearer.token(), &KEYS.decoding, &validation)
            .map_err(|_| AuthError::InvalidToken)?;

        Ok(token_data.claims)
    }
}

pub struct UserIp(pub String);
impl<S> FromRequestParts<S> for UserIp
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let real_ip = parts
            .headers
            .get("x-real-ip")
            .or_else(|| parts.headers.get("x-forwarded-for"))
            .and_then(|header| header.to_str().ok())
            .and_then(|header_str| header_str.split(',').next().map(|s| s.trim()));

        let real_ip = match real_ip {
            Some(real_ip) => real_ip,
            None => {
                let body = Json(json!({
                    "error": "Failed to fetch client ip address"
                }));
                return Err((StatusCode::FORBIDDEN, body).into_response());
            }
        };

        Ok(UserIp(real_ip.to_string()))
    }
}

pub fn get_claims(token: &str) -> Result<Claims, AuthError> {
    let validation = Validation::new(Algorithm::EdDSA);
    let token_data = decode::<Claims>(token, &KEYS.decoding, &validation)
        .map_err(|_| AuthError::InvalidToken)?;
    Ok(token_data.claims)
}

pub fn token_is_valid(token: &str) -> bool {
    let validation = Validation::new(Algorithm::EdDSA);
    let token_data = decode::<Claims>(token, &KEYS.decoding, &validation);
    return token_data.is_ok();
}

pub async fn guard(
    Extension(state): Extension<Arc<AppState>>,
    TypedHeader(bearer): TypedHeader<Authorization<Bearer>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if token_is_valid(bearer.token()) {
        let validation = Validation::new(Algorithm::EdDSA);
        let token_data = decode::<Claims>(bearer.token(), &KEYS.decoding, &validation).unwrap();
        if !token_data.claims.whitelisted {
            Err(StatusCode::UNAUTHORIZED)
        } else {
            let current_logged_in_user = state.get_db().get_user_by_id(token_data.claims.id).await;
            let current_logged_in_user = match current_logged_in_user {
                Some(user) => user,
                None => {
                    return Err(StatusCode::UNAUTHORIZED);
                }
            };

            if !current_logged_in_user.whitelisted {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let response = next.run(request).await;
            Ok(response)
        }
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub async fn guard_admin(
    TypedHeader(bearer): TypedHeader<Authorization<Bearer>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if token_is_valid(bearer.token()) {
        let validation = Validation::new(Algorithm::EdDSA);
        let token_data = decode::<Claims>(bearer.token(), &KEYS.decoding, &validation).unwrap();
        if !token_data.claims.whitelisted {
            Err(StatusCode::UNAUTHORIZED)
        } else {
            if token_data.claims.group_id == 1 {
                let response = next.run(request).await;
                Ok(response)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
