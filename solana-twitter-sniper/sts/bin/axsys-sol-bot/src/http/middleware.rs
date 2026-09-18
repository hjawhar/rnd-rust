use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};

pub async fn guard(request: Request, next: Next) -> Result<Response, StatusCode> {
    let current_x_api_key = std::env::var("X_API_KEY").expect("X_API_KEY is required");
    let api_key = request.headers().get("x-api-key");
    match api_key {
        Some(api_key) => {
            if api_key.is_empty() {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let api_key = match api_key.to_str() {
                Ok(api_key) => api_key,
                Err(_) => {
                    return Err(StatusCode::UNAUTHORIZED);
                }
            };
            if api_key != current_x_api_key {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let response = next.run(request).await;
            Ok(response)
        }
        None => Err(StatusCode::UNAUTHORIZED),
    }
}
