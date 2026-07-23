//! SeaORM transaction execution for the SQLite infrastructure adapter.

use std::{future::Future, pin::Pin};

use agena_storage::TransactionEffects;
use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, TransactionTrait};

/// Runs a SQLite/SeaORM transaction and executes queued effects only after commit.
pub async fn run_transaction_effects<T, O>(db: &DatabaseConnection, op: O) -> Result<T, DbErr>
where
    O: for<'a> FnOnce(
        &'a DatabaseTransaction,
        &'a mut TransactionEffects,
    ) -> Pin<Box<dyn Future<Output = Result<T, DbErr>> + Send + 'a>>,
{
    let transaction = db.begin().await?;
    let mut effects = TransactionEffects::new();
    match op(&transaction, &mut effects).await {
        Ok(value) => {
            transaction.commit().await?;
            effects.run().await;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().await?;
            Err(error)
        }
    }
}

/// Error-generic variant for application-facing transaction choreography.
pub async fn run_transaction_app_effects<T, E, O>(db: &DatabaseConnection, op: O) -> Result<T, E>
where
    E: From<DbErr>,
    O: for<'a> FnOnce(
        &'a DatabaseTransaction,
        &'a mut TransactionEffects,
    ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>,
{
    let transaction = db.begin().await.map_err(E::from)?;
    let mut effects = TransactionEffects::new();
    match op(&transaction, &mut effects).await {
        Ok(value) => {
            transaction.commit().await.map_err(E::from)?;
            effects.run().await;
            Ok(value)
        }
        Err(error) => {
            transaction.rollback().await.map_err(E::from)?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use sea_orm::Database;

    use super::{DbErr, run_transaction_effects};

    #[tokio::test]
    async fn effects_run_only_after_a_successful_commit() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("open database");
        let committed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&committed);
        run_transaction_effects(&db, move |_transaction, effects| {
            effects.push(async move { flag.store(true, Ordering::SeqCst) });
            Box::pin(async { Ok(()) })
        })
        .await
        .expect("commit transaction");
        assert!(committed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn effects_do_not_run_after_a_rollback() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("open database");
        let committed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&committed);
        let result: Result<(), DbErr> =
            run_transaction_effects(&db, move |_transaction, effects| {
                effects.push(async move { flag.store(true, Ordering::SeqCst) });
                Box::pin(async { Err(DbErr::Custom("abort transaction".to_owned())) })
            })
            .await;
        assert!(result.is_err());
        assert!(!committed.load(Ordering::SeqCst));
    }
}
