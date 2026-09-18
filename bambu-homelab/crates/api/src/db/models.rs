use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;
use uuid::Uuid;

use super::schema::{filament_usage, print_jobs, print_queue, printer_assignments, printers, users};

#[derive(Debug, Queryable, Selectable, Serialize)]
#[diesel(table_name = users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = users)]
pub struct NewUser<'a> {
    pub username: &'a str,
    pub password_hash: &'a str,
    pub role: &'a str,
}

#[derive(Debug, Queryable, Selectable, Serialize, Clone)]
#[diesel(table_name = printers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Printer {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub serial: String,
    pub access_code: String,
    pub model: String,
    pub owner_id: Uuid,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = printers)]
pub struct NewPrinter<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub ip: &'a str,
    pub serial: &'a str,
    pub access_code: &'a str,
    pub model: &'a str,
    pub owner_id: &'a Uuid,
}


#[derive(Debug, Queryable, Selectable, Serialize, Clone)]
#[diesel(table_name = print_jobs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PrintJob {
    pub id: Uuid,
    pub printer_id: String,
    pub file_name: String,
    pub started_at: NaiveDateTime,
    pub finished_at: Option<NaiveDateTime>,
    pub status: String,
    pub total_layers: i32,
    pub duration_seconds: Option<i32>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = print_jobs)]
pub struct NewPrintJob<'a> {
    pub printer_id: &'a str,
    pub file_name: &'a str,
    pub status: &'a str,
    pub total_layers: i32,
}

#[derive(Debug, Queryable, Selectable, Serialize, Clone)]
#[diesel(table_name = print_queue)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct QueueItem {
    pub id: Uuid,
    pub printer_id: String,
    pub file_name: String,
    pub plate_number: i32,
    pub status: String,
    pub position: i32,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = print_queue)]
pub struct NewQueueItem<'a> {
    pub printer_id: &'a str,
    pub file_name: &'a str,
    pub plate_number: i32,
    pub position: i32,
}

#[derive(Debug, Queryable, Selectable, Serialize, Clone)]
#[diesel(table_name = filament_usage)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FilamentUsage {
    pub id: Uuid,
    pub print_job_id: Option<Uuid>,
    pub printer_id: String,
    pub filament_type: String,
    pub color: String,
    pub weight_grams: Option<f32>,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Queryable, Selectable, Serialize, Clone)]
#[diesel(table_name = printer_assignments)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PrinterAssignment {
    pub id: Uuid,
    pub user_id: Uuid,
    pub printer_id: String,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = printer_assignments)]
pub struct NewPrinterAssignment<'a> {
    pub user_id: &'a Uuid,
    pub printer_id: &'a str,
}