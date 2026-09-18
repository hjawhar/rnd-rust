use super::*;
use crate::db::schema::{payments, projects, subscriptions};
use crate::models::payment::{NewPayment, Payment};
use crate::models::subscription::{NewSubscription, Subscription};
use crate::models::project::Project;

impl Database {
    // ── Subscriptions ───────────────────────────────────────────────

    pub async fn create_subscription(&self, sub: &NewSubscription) -> DbResult<Subscription> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let result = diesel::insert_into(subscriptions::table)
            .values(sub)
            .returning(Subscription::as_returning())
            .get_result(connection)
            .await?;
        Ok(result)
    }

    pub async fn get_subscriptions_by_project_ids(&self, project_ids: &[i32]) -> DbResult<Vec<Subscription>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let result = subscriptions::table
            .filter(subscriptions::project_id.eq_any(project_ids))
            .load(connection)
            .await?;
        Ok(result)
    }

    pub async fn get_subscription_by_project(&self, project_id: i32) -> DbResult<Option<Subscription>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let result = subscriptions::table
            .filter(subscriptions::project_id.eq(project_id))
            .first(connection)
            .await
            .optional()?;
        Ok(result)
    }

    pub async fn get_all_subscriptions(&self) -> DbResult<Vec<(Subscription, Project)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let result = subscriptions::table
            .inner_join(projects::table)
            .select((Subscription::as_returning(), Project::as_returning()))
            .order(subscriptions::next_payment_due.asc())
            .load(connection)
            .await?;
        Ok(result)
    }

    pub async fn get_overdue_subscriptions(&self) -> DbResult<Vec<(Subscription, Project)>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let now = SystemTime::now();
        let result = subscriptions::table
            .inner_join(projects::table)
            .filter(
                subscriptions::next_payment_due.lt(now)
                    .and(subscriptions::status.ne("cancelled")),
            )
            .select((Subscription::as_returning(), Project::as_returning()))
            .order(subscriptions::next_payment_due.asc())
            .load(connection)
            .await?;
        Ok(result)
    }

    pub async fn update_subscription_fields(
        &self,
        sub_id: i32,
        monthly_rate: Option<BigDecimal>,
        currency: Option<String>,
        next_payment_due: Option<SystemTime>,
        status: Option<String>,
        notes: Option<String>,
    ) -> DbResult<Option<Subscription>> {
        #[derive(AsChangeset)]
        #[diesel(table_name = subscriptions)]
        #[diesel(treat_none_as_null = false)]
        struct UpdateSubscriptionChangeset {
            monthly_rate: Option<BigDecimal>,
            currency: Option<String>,
            status: Option<String>,
            notes: Option<String>,
            next_payment_due: Option<SystemTime>,
        }

        let changeset = UpdateSubscriptionChangeset {
            monthly_rate,
            currency,
            status,
            notes,
            next_payment_due,
        };

        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        diesel::update(subscriptions::table.filter(subscriptions::id.eq(sub_id)))
            .set(&changeset)
            .execute(connection)
            .await?;

        let result = subscriptions::table
            .filter(subscriptions::id.eq(sub_id))
            .first(connection)
            .await
            .optional()?;
        Ok(result)
    }

    pub async fn advance_subscription_due_date(&self, sub_id: i32) -> DbResult<Option<Subscription>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        // Advance next_payment_due by ~30 days (30 * 86400 seconds)
        let thirty_days = std::time::Duration::from_secs(30 * 86400);

        let sub: Option<Subscription> = subscriptions::table
            .filter(subscriptions::id.eq(sub_id))
            .first(connection)
            .await
            .optional()?;

        if let Some(ref s) = sub {
            let new_due = s.next_payment_due + thirty_days;
            diesel::update(subscriptions::table.filter(subscriptions::id.eq(sub_id)))
                .set((
                    subscriptions::next_payment_due.eq(new_due),
                    subscriptions::status.eq("active"),
                ))
                .execute(connection)
                .await?;
            // Return updated
            let updated = subscriptions::table
                .filter(subscriptions::id.eq(sub_id))
                .first(connection)
                .await
                .optional()?;
            return Ok(updated);
        }
        Ok(None)
    }

    pub async fn delete_subscription(&self, sub_id: i32) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let deleted = diesel::delete(subscriptions::table.filter(subscriptions::id.eq(sub_id)))
            .execute(connection)
            .await?;
        Ok(deleted > 0)
    }

    // ── Payments ────────────────────────────────────────────────────

    pub async fn create_payment(&self, payment: &NewPayment) -> DbResult<Payment> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let result = diesel::insert_into(payments::table)
            .values(payment)
            .returning(Payment::as_returning())
            .get_result(connection)
            .await?;
        Ok(result)
    }

    pub async fn get_payments_by_subscription(&self, subscription_id: i32) -> DbResult<Vec<Payment>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let result = payments::table
            .filter(payments::subscription_id.eq(subscription_id))
            .order(payments::paid_at.desc())
            .load(connection)
            .await?;
        Ok(result)
    }

    pub async fn get_subscription_total_paid(&self, subscription_id: i32) -> DbResult<BigDecimal> {
        use diesel::dsl::sum;
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let total: Option<BigDecimal> = payments::table
            .filter(payments::subscription_id.eq(subscription_id))
            .select(sum(payments::amount))
            .first(connection)
            .await?;
        Ok(total.unwrap_or_else(|| BigDecimal::from(0)))
    }

    pub async fn delete_payment(&self, payment_id: i32) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let deleted = diesel::delete(payments::table.filter(payments::id.eq(payment_id)))
            .execute(connection)
            .await?;
        Ok(deleted > 0)
    }
}
