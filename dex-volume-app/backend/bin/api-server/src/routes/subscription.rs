use std::sync::Arc;
use std::time::SystemTime;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use vm_data::models::payment::{CreatePaymentPayload, NewPayment};
use vm_data::models::subscription::{
    CreateSubscriptionPayload, NewSubscription, UpdateSubscriptionPayload,
};
use vm_data::utils::helpers::f64_to_big_int;
use serde_json::json;

use crate::models::state::AppState;
use crate::routes::middleware::Claims;

fn parse_date(s: &str) -> Option<SystemTime> {
    // Accepts "YYYY-MM-DD" format
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i64 = parts[0].parse().ok()?;
    let month: i64 = parts[1].parse().ok()?;
    let day: i64 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Convert to days since Unix epoch
    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let days_in_month: [i64; 13] = [0, 31, if is_leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if day > days_in_month[month as usize] {
        return None;
    }
    // Days from year 0 to year, then subtract epoch
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let days = 365 * y + y / 4 - y / 100 + y / 400 + (m * 306 + 5) / 10 + (day - 1) - 719468;
    if days < 0 {
        return None;
    }
    Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(days as u64 * 86400))
}

fn days_overdue(next_payment_due: SystemTime) -> i64 {
    let now = SystemTime::now();
    match now.duration_since(next_payment_due) {
        Ok(d) => (d.as_secs() / 86400) as i64,
        Err(_) => 0,
    }
}

use super::helpers::audit_log;

// ── Admin endpoints ─────────────────────────────────────────────

/// POST /admin/projects/{id}/subscription
pub async fn create_subscription_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<i32>,
    Json(payload): Json<CreateSubscriptionPayload>,
) -> impl IntoResponse {
    // Verify project exists
    let projects = state.get_db().get_all_projects().await.unwrap_or_default();
    if !projects.iter().any(|p| p.id == project_id) {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))).into_response();
    }

    // Check if subscription already exists
    if let Ok(Some(_)) = state.get_db().get_subscription_by_project(project_id).await {
        return (StatusCode::CONFLICT, Json(json!({ "error": "Subscription already exists for this project" }))).into_response();
    }

    let next_due = match parse_date(&payload.next_payment_due) {
        Some(d) => d,
        None => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid date format. Use YYYY-MM-DD" }))).into_response();
        }
    };

    if payload.monthly_rate <= 0.0 {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Monthly rate must be positive" }))).into_response();
    }

    let new_sub = NewSubscription {
        project_id,
        monthly_rate: f64_to_big_int(payload.monthly_rate),
        currency: payload.currency,
        started_at: SystemTime::now(),
        next_payment_due: next_due,
        status: "active".to_string(),
        notes: payload.notes,
    };

    match state.get_db().create_subscription(&new_sub).await {
        Ok(sub) => {
            audit_log(&state, claims.id, Some(project_id), "subscription.create", Some(json!({
                "monthly_rate": sub.monthly_rate.to_string(),
                "currency": sub.currency,
            })));
            (StatusCode::OK, Json(json!({ "data": sub }))).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to create subscription: {}", e) }))).into_response()
        }
    }
}

/// PATCH /admin/projects/{id}/subscription
pub async fn update_subscription_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<i32>,
    Json(payload): Json<UpdateSubscriptionPayload>,
) -> impl IntoResponse {
    let sub = match state.get_db().get_subscription_by_project(project_id).await.ok().flatten() {
        Some(s) => s,
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Subscription not found" })));
        }
    };

    if let Some(ref s) = payload.status
        && !["active", "overdue", "suspended", "cancelled"].contains(&s.as_str()) {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid status. Must be: active, overdue, suspended, cancelled" })));
        }

    let monthly_rate = payload.monthly_rate.map(f64_to_big_int);
    let next_due = match payload.next_payment_due {
        Some(ref d) => match parse_date(d) {
            Some(dt) => Some(dt),
            None => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid date format. Use YYYY-MM-DD" })));
            }
        },
        None => None,
    };

    match state.get_db().update_subscription_fields(
        sub.id,
        monthly_rate,
        payload.currency,
        next_due,
        payload.status,
        payload.notes,
    ).await {
        Ok(Some(updated)) => {
            audit_log(&state, claims.id, Some(project_id), "subscription.update", None);
            (StatusCode::OK, Json(json!({ "data": updated })))
        }
        Ok(None) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "Subscription not found" })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to update: {}", e) })))
        }
    }
}

/// DELETE /admin/projects/{id}/subscription
pub async fn delete_subscription_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<i32>,
) -> impl IntoResponse {
    let sub = match state.get_db().get_subscription_by_project(project_id).await.ok().flatten() {
        Some(s) => s,
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Subscription not found" })));
        }
    };

    match state.get_db().delete_subscription(sub.id).await {
        Ok(true) => {
            audit_log(&state, claims.id, Some(project_id), "subscription.delete", None);
            (StatusCode::OK, Json(json!({ "message": "Subscription deleted" })))
        }
        Ok(false) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "Subscription not found" })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to delete: {}", e) })))
        }
    }
}

/// GET /admin/subscriptions
pub async fn get_all_subscriptions_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.read_db().get_all_subscriptions().await {
        Ok(subs) => {
            let data: Vec<serde_json::Value> = subs.iter().map(|(sub, project)| {
                let overdue = days_overdue(sub.next_payment_due);
                json!({
                    "subscription": sub,
                    "project": {
                        "id": project.id,
                        "name": project.name,
                        "symbol": project.symbol,
                        "network": project.network,
                        "user_id": project.user_id,
                    },
                    "days_overdue": overdue,
                    "is_overdue": overdue > 0 && sub.status != "cancelled",
                })
            }).collect();
            (StatusCode::OK, Json(json!({ "data": data })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to fetch subscriptions: {}", e) })))
        }
    }
}

/// GET /admin/subscriptions/overdue
pub async fn get_overdue_subscriptions_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.read_db().get_overdue_subscriptions().await {
        Ok(subs) => {
            let data: Vec<serde_json::Value> = subs.iter().map(|(sub, project)| {
                json!({
                    "subscription": sub,
                    "project": {
                        "id": project.id,
                        "name": project.name,
                        "symbol": project.symbol,
                        "network": project.network,
                        "user_id": project.user_id,
                    },
                    "days_overdue": days_overdue(sub.next_payment_due),
                })
            }).collect();
            (StatusCode::OK, Json(json!({ "data": data })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to fetch overdue: {}", e) })))
        }
    }
}

/// POST /admin/subscriptions/{id}/payments
pub async fn create_payment_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(subscription_id): Path<i32>,
    Json(payload): Json<CreatePaymentPayload>,
) -> impl IntoResponse {
    if payload.amount <= 0.0 {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Amount must be positive" }))).into_response();
    }

    let paid_at = match payload.paid_at {
        Some(ref d) => match parse_date(d) {
            Some(dt) => dt,
            None => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid date format. Use YYYY-MM-DD" }))).into_response();
            }
        },
        None => SystemTime::now(),
    };

    let new_payment = NewPayment {
        subscription_id,
        amount: f64_to_big_int(payload.amount),
        currency: payload.currency,
        paid_at,
        recorded_by: claims.id,
        notes: payload.notes,
    };

    match state.get_db().create_payment(&new_payment).await {
        Ok(payment) => {
            // Auto-advance the due date
            let _ = state.get_db().advance_subscription_due_date(subscription_id).await;
            audit_log(&state, claims.id, None, "payment.record", Some(json!({
                "subscription_id": subscription_id,
                "amount": payment.amount.to_string(),
            })));
            (StatusCode::OK, Json(json!({ "data": payment }))).into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("foreign key") || msg.contains("violates") {
                (StatusCode::NOT_FOUND, Json(json!({ "error": "Subscription not found" }))).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to record payment: {}", msg) }))).into_response()
            }
        }
    }
}

/// GET /admin/subscriptions/{id}/payments
pub async fn get_payments_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(subscription_id): Path<i32>,
) -> impl IntoResponse {
    let payments = state.read_db().get_payments_by_subscription(subscription_id).await.unwrap_or_default();
    let total = state.read_db().get_subscription_total_paid(subscription_id).await.unwrap_or_else(|_| bigdecimal::BigDecimal::from(0));
    (StatusCode::OK, Json(json!({
        "data": {
            "payments": payments,
            "total_paid": total,
        }
    })))
}

/// DELETE /admin/subscriptions/{id}/payments/{payment_id}
pub async fn delete_payment_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path((subscription_id, payment_id)): Path<(i32, i32)>,
) -> impl IntoResponse {
    match state.get_db().delete_payment(payment_id).await {
        Ok(true) => {
            audit_log(&state, claims.id, None, "payment.void", Some(json!({
                "subscription_id": subscription_id,
                "payment_id": payment_id,
            })));
            (StatusCode::OK, Json(json!({ "message": "Payment voided" })))
        }
        Ok(false) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "Payment not found" })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to delete payment: {}", e) })))
        }
    }
}

// ── User endpoint ───────────────────────────────────────────────

/// GET /subscriptions — all subscriptions for the user's projects (single call)
pub async fn get_my_subscriptions_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let projects = match state.read_db().get_projects(claims.id).await {
        Ok(p) => p,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to fetch projects: {}", e) })));
        }
    };

    let project_ids: Vec<i32> = projects.iter().map(|p| p.id).collect();
    let subs = state.read_db().get_subscriptions_by_project_ids(&project_ids).await.unwrap_or_default();

    let data: serde_json::Map<String, serde_json::Value> = subs.into_iter().map(|sub| {
        let overdue = days_overdue(sub.next_payment_due);
        let info = json!({
            "monthly_rate": sub.monthly_rate,
            "currency": sub.currency,
            "next_payment_due": sub.next_payment_due,
            "status": sub.status,
            "started_at": sub.started_at,
            "days_overdue": overdue,
            "is_overdue": overdue > 0 && sub.status != "cancelled",
        });
        (sub.project_id.to_string(), info)
    }).collect();

    (StatusCode::OK, Json(json!({ "data": data })))
}

/// GET /projects/{id}/subscription — user-visible subscription info
pub async fn get_project_subscription_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<i32>,
) -> impl IntoResponse {
    // Verify user has access to the project
    let project = match state.read_db().get_project(claims.id, project_id).await.ok().flatten() {
        Some(p) => p,
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" })));
        }
    };

    match state.read_db().get_subscription_by_project(project.id).await {
        Ok(Some(sub)) => {
            let overdue = days_overdue(sub.next_payment_due);
            // User sees limited info — no admin notes
            (StatusCode::OK, Json(json!({
                "data": {
                    "monthly_rate": sub.monthly_rate,
                    "currency": sub.currency,
                    "next_payment_due": sub.next_payment_due,
                    "status": sub.status,
                    "started_at": sub.started_at,
                    "days_overdue": overdue,
                    "is_overdue": overdue > 0 && sub.status != "cancelled",
                }
            })))
        }
        Ok(None) => {
            (StatusCode::OK, Json(json!({ "data": null })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to fetch subscription: {}", e) })))
        }
    }
}
