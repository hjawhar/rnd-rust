pub mod models;
pub mod schema;

use diesel_async::pooled_connection::bb8;
use diesel_async::AsyncPgConnection;

pub type Pool = bb8::Pool<AsyncPgConnection>;
